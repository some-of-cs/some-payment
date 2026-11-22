//! Raw webhook byte-cap validation, case-insensitive header lookup helpers,
//! and query parameter parsing.
//!
//! Enforces payload size and map-cardinality bounds *before* any JSON parsing,
//! cryptographic signature verification, or database lookup (US-3.2.0, `TM-5`).

use std::collections::BTreeMap;
use std::fmt;

use some_payment_core::{HttpMethod, PaymentError, RawWebhookRequest};

/// Default maximum permitted webhook HTTP body size in bytes (1 MiB, `TM-5`).
pub const MAX_WEBHOOK_BYTES: usize = 1_048_576;

/// Maximum permitted number of HTTP headers on an incoming webhook request.
pub const MAX_HEADER_COUNT: usize = 128;

/// Maximum permitted number of URL query parameters on an incoming webhook request.
pub const MAX_QUERY_PARAM_COUNT: usize = 128;

/// A [`RawWebhookRequest`] that has passed pre-verification byte-cap and
/// header/query cardinality bounds checks.
///
/// Implements custom [`fmt::Debug`] delegating to [`RawWebhookRequest`]'s
/// redacted formatter so raw webhook bytes, header values, and query parameter
/// values are never emitted in logs or debug output.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use some_payment::webhooks::raw_request::{MAX_WEBHOOK_BYTES, ValidatedRawWebhook};
/// use some_payment_core::{HttpMethod, RawWebhookRequest};
///
/// let mut headers = BTreeMap::new();
/// headers.insert("Stripe-Signature".into(), "t=1,v1=sig".into());
/// let raw = RawWebhookRequest {
///     raw_body: b"{}".to_vec(),
///     headers,
///     query_params: BTreeMap::new(),
///     method: HttpMethod::Post,
/// };
///
/// let validated = ValidatedRawWebhook::try_new(raw).unwrap();
/// assert_eq!(validated.body_len(), 2);
/// assert!(validated.body_len() <= MAX_WEBHOOK_BYTES);
/// assert_eq!(validated.header("stripe-signature"), Some("t=1,v1=sig"));
/// assert_eq!(validated.method(), HttpMethod::Post);
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct ValidatedRawWebhook {
    raw: RawWebhookRequest,
}

impl ValidatedRawWebhook {
    /// Validates `raw` against [`MAX_WEBHOOK_BYTES`], [`MAX_HEADER_COUNT`], and
    /// [`MAX_QUERY_PARAM_COUNT`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if the body exceeds
    /// [`MAX_WEBHOOK_BYTES`], headers exceed [`MAX_HEADER_COUNT`], or query
    /// parameters exceed [`MAX_QUERY_PARAM_COUNT`].
    pub fn try_new(raw: RawWebhookRequest) -> Result<Self, PaymentError> {
        Self::try_with_limit(raw, MAX_WEBHOOK_BYTES)
    }

    /// Validates `raw` against a custom `max_bytes` body cap along with
    /// [`MAX_HEADER_COUNT`] and [`MAX_QUERY_PARAM_COUNT`].
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `max_bytes == 0`, the body
    /// exceeds `max_bytes`, headers exceed [`MAX_HEADER_COUNT`], or query
    /// parameters exceed [`MAX_QUERY_PARAM_COUNT`].
    pub fn try_with_limit(raw: RawWebhookRequest, max_bytes: usize) -> Result<Self, PaymentError> {
        validate_byte_limit(raw.raw_body.len(), max_bytes)?;
        validate_map_bounds(raw.headers.len(), raw.query_params.len())?;
        Ok(Self { raw })
    }

    /// Returns a shared reference to the underlying [`RawWebhookRequest`].
    #[must_use]
    pub fn as_raw(&self) -> &RawWebhookRequest {
        &self.raw
    }

    /// Consumes the guard and returns the underlying [`RawWebhookRequest`].
    #[must_use]
    pub fn into_inner(self) -> RawWebhookRequest {
        self.raw
    }

    /// Performs an ASCII case-insensitive HTTP header lookup.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.raw.header(name)
    }

    /// Looks up a URL query parameter by exact key.
    #[must_use]
    pub fn query(&self, key: &str) -> Option<&str> {
        self.raw.query(key)
    }

    /// Returns the byte length of the raw HTTP body.
    #[must_use]
    pub fn body_len(&self) -> usize {
        self.raw.raw_body.len()
    }

    /// Returns the HTTP method of the incoming webhook request.
    #[must_use]
    pub fn method(&self) -> HttpMethod {
        self.raw.method
    }
}

impl fmt::Debug for ValidatedRawWebhook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatedRawWebhook")
            .field("raw", &self.raw)
            .finish()
    }
}

/// Parses a URL query string into a sorted map of decoded key-value pairs.
///
/// Strips an optional leading `?`, enforces [`MAX_QUERY_PARAM_COUNT`], splits
/// pairs on `&` and the first `=`, and percent-decodes keys and values safely
/// without panics or `unsafe` code.
///
/// # Errors
///
/// Returns [`PaymentError::ValidationError`] if the parameter count exceeds
/// [`MAX_QUERY_PARAM_COUNT`], a key is empty, a percent-escape sequence is
/// malformed, or the decoded bytes are not valid UTF-8.
///
/// # Examples
///
/// ```
/// use some_payment::webhooks::raw_request::parse_query_string;
///
/// let params = parse_query_string("?hmac=deadbeef&id=12345&note=hello%20world").unwrap();
/// assert_eq!(params.get("hmac").map(String::as_str), Some("deadbeef"));
/// assert_eq!(params.get("id").map(String::as_str), Some("12345"));
/// assert_eq!(params.get("note").map(String::as_str), Some("hello world"));
/// ```
pub fn parse_query_string(raw_query: &str) -> Result<BTreeMap<String, String>, PaymentError> {
    let query = raw_query.strip_prefix('?').unwrap_or(raw_query);
    let mut params = BTreeMap::new();
    for (index, segment) in query.split('&').filter(|s| !s.is_empty()).enumerate() {
        if index >= MAX_QUERY_PARAM_COUNT {
            return Err(PaymentError::validation(format!(
                "webhook query count exceeds limit ({MAX_QUERY_PARAM_COUNT})"
            )));
        }
        let (key, value) = parse_query_pair(segment)?;
        params.insert(key, value);
    }
    Ok(params)
}

fn validate_byte_limit(body_len: usize, max_bytes: usize) -> Result<(), PaymentError> {
    if max_bytes == 0 {
        return Err(PaymentError::validation(
            "webhook maximum body byte limit must be greater than zero",
        ));
    }
    if body_len > max_bytes {
        return Err(PaymentError::validation(format!(
            "webhook body ({body_len} bytes) exceeds limit ({max_bytes} bytes)"
        )));
    }
    Ok(())
}

fn validate_map_bounds(header_count: usize, query_count: usize) -> Result<(), PaymentError> {
    if header_count > MAX_HEADER_COUNT {
        return Err(PaymentError::validation(format!(
            "webhook header count ({header_count}) exceeds limit ({MAX_HEADER_COUNT})"
        )));
    }
    if query_count > MAX_QUERY_PARAM_COUNT {
        return Err(PaymentError::validation(format!(
            "webhook query count ({query_count}) exceeds limit ({MAX_QUERY_PARAM_COUNT})"
        )));
    }
    Ok(())
}

fn parse_query_pair(segment: &str) -> Result<(String, String), PaymentError> {
    let (raw_key, raw_value) = segment.split_once('=').unwrap_or((segment, ""));
    let key = percent_decode(raw_key)?;
    if key.is_empty() {
        return Err(PaymentError::validation(
            "webhook query parameter key must not be empty",
        ));
    }
    let value = percent_decode(raw_value)?;
    Ok((key, value))
}

fn percent_decode(input: &str) -> Result<String, PaymentError> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0;

    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            decoded.push(decode_percent_escape(bytes, idx)?);
            idx += 3;
        } else {
            decoded.push(bytes[idx]);
            idx += 1;
        }
    }

    String::from_utf8(decoded)
        .map_err(|_| PaymentError::validation("webhook query parameter is not valid UTF-8"))
}

fn decode_percent_escape(bytes: &[u8], start: usize) -> Result<u8, PaymentError> {
    let hi_byte = bytes.get(start + 1).copied();
    let lo_byte = bytes.get(start + 2).copied();
    match (hi_byte.and_then(hex_nibble), lo_byte.and_then(hex_nibble)) {
        (Some(hi), Some(lo)) => Ok((hi << 4) | lo),
        _ => Err(PaymentError::validation(
            "malformed percent-encoding in webhook query string",
        )),
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use some_payment_core::ErrorKind;

    fn sample_request(body: Vec<u8>) -> RawWebhookRequest {
        RawWebhookRequest {
            raw_body: body,
            headers: BTreeMap::new(),
            query_params: BTreeMap::new(),
            method: HttpMethod::Post,
        }
    }

    #[test]
    fn tc_3_2_1_exact_1_mib_boundary_passes() {
        let req = sample_request(vec![b'a'; MAX_WEBHOOK_BYTES]);
        let validated = ValidatedRawWebhook::try_new(req).expect("exact 1 MiB body must pass");
        assert_eq!(validated.body_len(), MAX_WEBHOOK_BYTES);
        assert_eq!(validated.as_raw().raw_body.len(), MAX_WEBHOOK_BYTES);
        assert_eq!(validated.method(), HttpMethod::Post);

        let inner = validated.into_inner();
        assert_eq!(inner.raw_body.len(), MAX_WEBHOOK_BYTES);
    }

    #[test]
    fn tc_3_2_1_oversized_body_rejected_before_parsing() {
        let req = sample_request(vec![b'{'; MAX_WEBHOOK_BYTES + 1]);
        let err = ValidatedRawWebhook::try_new(req).expect_err("1 MiB + 1 byte must be rejected");
        assert_eq!(err.kind(), ErrorKind::Validation);
    }

    #[test]
    fn custom_byte_cap_enforced_and_zero_rejected() {
        let ok_req = sample_request(vec![b'x'; 64]);
        let validated = ValidatedRawWebhook::try_with_limit(ok_req, 64).expect("64-byte cap");
        assert_eq!(validated.body_len(), 64);

        let too_big = sample_request(vec![b'x'; 65]);
        let err = ValidatedRawWebhook::try_with_limit(too_big, 64).expect_err("65 > 64");
        assert_eq!(err.kind(), ErrorKind::Validation);

        let zero_err = ValidatedRawWebhook::try_with_limit(sample_request(Vec::new()), 0)
            .expect_err("zero cap must be rejected");
        assert_eq!(zero_err.kind(), ErrorKind::Validation);
    }

    #[test]
    fn case_insensitive_header_lookup() {
        let mut req = sample_request(b"{}".to_vec());
        req.headers.insert("Stripe-Signature".into(), "t=1700,v1=abc".into());
        let validated = ValidatedRawWebhook::try_new(req).unwrap();

        assert_eq!(validated.header("Stripe-Signature"), Some("t=1700,v1=abc"));
        assert_eq!(validated.header("stripe-signature"), Some("t=1700,v1=abc"));
        assert_eq!(validated.header("STRIPE-SIGNATURE"), Some("t=1700,v1=abc"));
        assert_eq!(validated.header("x-missing"), None);
    }

    #[test]
    fn header_and_query_count_limits_enforced() {
        let mut ok_req = sample_request(b"{}".to_vec());
        for i in 0..MAX_HEADER_COUNT {
            ok_req.headers.insert(format!("x-h-{i}"), "v".into());
        }
        for i in 0..MAX_QUERY_PARAM_COUNT {
            ok_req.query_params.insert(format!("q{i}"), "v".into());
        }
        assert!(ValidatedRawWebhook::try_new(ok_req).is_ok());

        let mut too_many_headers = sample_request(b"{}".to_vec());
        for i in 0..=MAX_HEADER_COUNT {
            too_many_headers.headers.insert(format!("x-h-{i}"), "v".into());
        }
        let h_err = ValidatedRawWebhook::try_new(too_many_headers).expect_err("129 headers");
        assert_eq!(h_err.kind(), ErrorKind::Validation);

        let mut too_many_query = sample_request(b"{}".to_vec());
        for i in 0..=MAX_QUERY_PARAM_COUNT {
            too_many_query.query_params.insert(format!("q{i}"), "v".into());
        }
        let q_err = ValidatedRawWebhook::try_new(too_many_query).expect_err("129 query params");
        assert_eq!(q_err.kind(), ErrorKind::Validation);
    }

    #[test]
    fn parse_query_string_paymob_and_edge_cases() {
        assert!(parse_query_string("").unwrap().is_empty());
        assert!(parse_query_string("?").unwrap().is_empty());

        let paymob = parse_query_string("?hmac=deadbeef01&id=98765&success=true").unwrap();
        assert_eq!(paymob.get("hmac").map(String::as_str), Some("deadbeef01"));
        assert_eq!(paymob.get("id").map(String::as_str), Some("98765"));
        assert_eq!(paymob.get("success").map(String::as_str), Some("true"));

        let decoded = parse_query_string("sig=a%3Db%20c%2Fd&flag&&raw=x=y").unwrap();
        assert_eq!(decoded.get("sig").map(String::as_str), Some("a=b c/d"));
        assert_eq!(decoded.get("flag").map(String::as_str), Some(""));
        assert_eq!(decoded.get("raw").map(String::as_str), Some("x=y"));
    }

    #[test]
    fn parse_query_string_rejects_malformed_escapes_and_overflow() {
        for bad in ["a=%", "a=%2", "a=%ZZ", "a=%2G", "a=%FF%FE", "=val"] {
            let err = parse_query_string(bad).expect_err("malformed query must fail");
            assert_eq!(err.kind(), ErrorKind::Validation);
        }

        let valid_128: Vec<String> = (0..MAX_QUERY_PARAM_COUNT)
            .map(|i| format!("k{i}=1"))
            .collect();
        assert_eq!(parse_query_string(&valid_128.join("&")).unwrap().len(), MAX_QUERY_PARAM_COUNT);

        let invalid_129: Vec<String> = (0..=MAX_QUERY_PARAM_COUNT)
            .map(|i| format!("k{i}=1"))
            .collect();
        let err = parse_query_string(&invalid_129.join("&")).expect_err("129 params must fail");
        assert_eq!(err.kind(), ErrorKind::Validation);
    }

    #[test]
    fn debug_redacts_body_headers_and_query_values() {
        let mut req = sample_request(b"{\"secret_token\":\"BODY_CANARY_123\"}".to_vec());
        req.headers.insert("Stripe-Signature".into(), "HEADER_CANARY_456".into());
        req.query_params.insert("hmac".into(), "QUERY_CANARY_789".into());
        req.method = HttpMethod::Get;

        let validated = ValidatedRawWebhook::try_new(req).unwrap();
        assert_eq!(validated.query("hmac"), Some("QUERY_CANARY_789"));
        assert_eq!(validated.method(), HttpMethod::Get);

        let dbg = format!("{validated:?}");
        assert!(!dbg.contains("BODY_CANARY_123"));
        assert!(!dbg.contains("HEADER_CANARY_456"));
        assert!(!dbg.contains("QUERY_CANARY_789"));
        assert!(dbg.contains("ValidatedRawWebhook"));
        assert!(dbg.contains("raw_body_len"));
        assert!(dbg.contains("[REDACTED]"));
    }
}
