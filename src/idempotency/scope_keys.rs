//! Namespaced scope-key builder `(tenant_id, provider_id, operation)`,
//! idempotency-key validator, and strict RFC 4122 / 9562 UUID v4 validator.
//!
//! This module mitigates cross-tenant idempotency key collisions (`TM-1`) by
//! enforcing colon-free, printable-ASCII namespace segments and provides strict
//! UUID v4 validation compatible with both Stripe `Idempotency-Key` headers
//! (`<= 255` bytes) and Moyasar `given_id` creation keys (`TC-3.1.1`).

use std::fmt;

use some_payment_core::{PaymentError, ProviderId};

/// Maximum allowed byte length for an idempotency key (compatible with Stripe
/// `Idempotency-Key` headers of `<= 255` bytes).
pub const MAX_IDEMPOTENCY_KEY_LEN: usize = 255;

/// Maximum allowed byte length for an individual scope segment (`tenant_id` or
/// `operation`).
pub const MAX_SCOPE_SEGMENT_LEN: usize = 64;

const UUID_V4_LEN: usize = 36;
const UUID_HYPHEN_INDICES: [usize; 4] = [8, 13, 18, 23];

/// Validated, collision-free namespaced idempotency scope.
///
/// Formats deterministically as `"{provider}:{operation}"` when unscoped by
/// tenant, or `"{tenant}:{provider}:{operation}"` when a tenant segment is
/// attached. Because colons, whitespace, control characters, empty strings, and
/// segments exceeding [`MAX_SCOPE_SEGMENT_LEN`] bytes are rejected, distinct
/// `(tenant_id, provider_id, operation)` tuples can never collide (`TM-1`).
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::scope_keys::IdempotencyScope;
/// use some_payment_core::ProviderId;
///
/// let provider = ProviderId::new("moyasar");
/// let unscoped = IdempotencyScope::try_new(&provider, "refund").unwrap();
/// assert_eq!(unscoped.as_str(), "moyasar:refund");
/// assert_eq!(unscoped.tenant_id(), None);
///
/// let scoped = unscoped.with_tenant("tenant-acme").unwrap();
/// assert_eq!(scoped.as_str(), "tenant-acme:moyasar:refund");
/// assert_eq!(scoped.tenant_id(), Some("tenant-acme"));
/// ```
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdempotencyScope {
    tenant_id: Option<String>,
    provider_id: ProviderId,
    operation: String,
    formatted: String,
}

impl IdempotencyScope {
    /// Builds a validated [`IdempotencyScope`] without a tenant segment
    /// (`"{provider}:{operation}"`).
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `operation` is empty,
    /// exceeds [`MAX_SCOPE_SEGMENT_LEN`] bytes, or contains whitespace, control
    /// characters, non-ASCII bytes, or `':'`.
    pub fn try_new(provider_id: &ProviderId, operation: &str) -> Result<Self, PaymentError> {
        validate_scope_segment("operation", operation)?;
        let provider = provider_id.as_str();
        let formatted = format!("{provider}:{operation}");
        Ok(Self {
            tenant_id: None,
            provider_id: provider_id.clone(),
            operation: operation.to_owned(),
            formatted,
        })
    }

    /// Builds a validated [`IdempotencyScope`] from an optional `tenant_id`,
    /// `provider_id`, and `operation`.
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `operation` or a provided
    /// `tenant_id` fails segment validation.
    pub fn from_parts(
        tenant_id: Option<&str>,
        provider_id: &ProviderId,
        operation: &str,
    ) -> Result<Self, PaymentError> {
        let scope = Self::try_new(provider_id, operation)?;
        match tenant_id {
            Some(tenant) => scope.with_tenant(tenant),
            None => Ok(scope),
        }
    }

    /// Attaches a validated `tenant_id` namespace segment, producing
    /// `"{tenant}:{provider}:{operation}"` to prevent cross-tenant collisions
    /// (`TM-1`).
    ///
    /// # Errors
    ///
    /// Returns [`PaymentError::ValidationError`] if `tenant_id` is empty,
    /// exceeds [`MAX_SCOPE_SEGMENT_LEN`] bytes, or contains whitespace, control
    /// characters, non-ASCII bytes, or `':'`.
    pub fn with_tenant(mut self, tenant_id: &str) -> Result<Self, PaymentError> {
        validate_scope_segment("tenant_id", tenant_id)?;
        let provider = self.provider_id.as_str();
        let operation = &self.operation;
        self.formatted = format!("{tenant_id}:{provider}:{operation}");
        self.tenant_id = Some(tenant_id.to_owned());
        Ok(self)
    }

    /// Returns the deterministic formatted scope string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.formatted
    }

    /// Returns the validated [`ProviderId`] component of this scope.
    #[must_use]
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    /// Returns the validated operation name component of this scope.
    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// Returns the optional validated tenant identifier, if set.
    #[must_use]
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}

impl fmt::Display for IdempotencyScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.formatted)
    }
}

impl AsRef<str> for IdempotencyScope {
    fn as_ref(&self) -> &str {
        &self.formatted
    }
}

impl fmt::Debug for IdempotencyScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdempotencyScope")
            .field("tenant_id", &self.tenant_id)
            .field("provider_id", &self.provider_id)
            .field("operation", &self.operation)
            .field("scope", &self.formatted)
            .finish()
    }
}

fn is_valid_scope_byte(byte: u8) -> bool {
    byte.is_ascii_graphic() && byte != b':'
}

fn validate_scope_segment(label: &'static str, segment: &str) -> Result<(), PaymentError> {
    if segment.is_empty() || segment.len() > MAX_SCOPE_SEGMENT_LEN {
        return Err(PaymentError::validation(format!(
            "idempotency {label} must be between 1 and {MAX_SCOPE_SEGMENT_LEN} bytes"
        )));
    }
    if !segment.bytes().all(is_valid_scope_byte) {
        return Err(PaymentError::validation(format!(
            "idempotency {label} must contain only printable ASCII without whitespace or ':'"
        )));
    }
    Ok(())
}

fn is_uuid_v4_variant_nibble(byte: u8) -> bool {
    matches!(byte, b'8' | b'9' | b'a' | b'b' | b'A' | b'B')
}

fn has_valid_uuid_layout(bytes: &[u8]) -> bool {
    bytes.iter().enumerate().all(|(idx, &byte)| {
        if UUID_HYPHEN_INDICES.contains(&idx) {
            byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    })
}

/// Validates that `key` is a strict canonical RFC 4122 / 9562 UUID v4 string
/// (`xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx`, 36 ASCII characters).
///
/// Accepts both lowercase and uppercase ASCII hexadecimal digits so creation
/// idempotency keys remain compatible with both Stripe `Idempotency-Key`
/// headers and Moyasar `given_id` parameters (`TC-3.1.1`).
///
/// # Errors
///
/// Returns [`PaymentError::ValidationError`] if `key` does not match the
/// canonical 36-character UUID v4 layout, version nibble (`4`), or variant
/// nibble (`8`, `9`, `a`/`A`, or `b`/`B`).
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::scope_keys::validate_uuid_v4;
///
/// assert!(validate_uuid_v4("550e8400-e29b-41d4-a716-446655440000").is_ok());
/// assert!(validate_uuid_v4("550e8400-e29b-11d4-a716-446655440000").is_err());
/// ```
pub fn validate_uuid_v4(key: &str) -> Result<(), PaymentError> {
    let bytes = key.as_bytes();
    if bytes.len() != UUID_V4_LEN || !has_valid_uuid_layout(bytes) {
        return Err(PaymentError::validation(
            "idempotency key must be a canonical 36-character RFC 4122/9562 UUID v4",
        ));
    }
    if bytes[14] != b'4' || !is_uuid_v4_variant_nibble(bytes[19]) {
        return Err(PaymentError::validation(
            "idempotency key must have UUID version 4 and RFC 4122/9562 variant [89ab]",
        ));
    }
    Ok(())
}

/// Validates a general idempotency key against length and printable-ASCII
/// bounds (`1..=255` bytes, printable ASCII without whitespace or control
/// characters).
///
/// Compatible with Stripe's `Idempotency-Key` header limit
/// ([`MAX_IDEMPOTENCY_KEY_LEN`]).
///
/// # Errors
///
/// Returns [`PaymentError::ValidationError`] if `key` is empty, exceeds
/// [`MAX_IDEMPOTENCY_KEY_LEN`] bytes, or contains whitespace, control
/// characters, or non-ASCII bytes.
///
/// # Examples
///
/// ```
/// use some_payment::idempotency::scope_keys::validate_idempotency_key;
///
/// assert!(validate_idempotency_key("refund:ord-1").is_ok());
/// assert!(validate_idempotency_key("").is_err());
/// assert!(validate_idempotency_key("key with space").is_err());
/// ```
pub fn validate_idempotency_key(key: &str) -> Result<(), PaymentError> {
    if key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_LEN {
        return Err(PaymentError::validation(format!(
            "idempotency key must be between 1 and {MAX_IDEMPOTENCY_KEY_LEN} bytes"
        )));
    }
    if !key.as_bytes().iter().all(u8::is_ascii_graphic) {
        return Err(PaymentError::validation(
            "idempotency key must contain only printable ASCII without whitespace or control chars",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use some_payment_core::ErrorKind;

    use super::*;

    fn assert_all_rejected<T: fmt::Debug>(
        inputs: &[String],
        validate: impl Fn(&str) -> Result<T, PaymentError>,
    ) {
        for input in inputs {
            let err = validate(input).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation);
        }
    }

    fn build_invalid_segments(prefix: &str, max_len: usize, include_colon: bool) -> Vec<String> {
        let mut samples = vec![
            String::new(),
            " ".to_owned(),
            format!("{prefix} 1"),
            format!("{prefix}\t1"),
            format!("{prefix}\n1"),
            format!("{prefix}\r1"),
            format!("{prefix}\x001"),
            format!("{prefix}\x1b1"),
            format!("{prefix}\x7f1"),
            format!("{prefix}-é"),
            "x".repeat(max_len + 1),
        ];
        if include_colon {
            samples.push(format!("{prefix}:sub"));
            samples.push(":".to_owned());
        }
        samples
    }

    #[test]
    fn scope_formats_deterministically_without_tenant() {
        let provider = ProviderId::new("moyasar");
        let scope = IdempotencyScope::try_new(&provider, "refund").unwrap();
        assert_eq!(
            (scope.as_str(), scope.to_string().as_str(), scope.as_ref()),
            ("moyasar:refund", "moyasar:refund", "moyasar:refund")
        );
        assert_eq!(
            (scope.provider_id(), scope.operation(), scope.tenant_id()),
            (&provider, "refund", None)
        );
        assert!(format!("{scope:?}").contains("IdempotencyScope"));
    }

    #[test]
    fn scope_formats_deterministically_with_tenant_and_from_parts() {
        let provider = ProviderId::new("paymob");
        let via_builder = IdempotencyScope::try_new(&provider, "capture")
            .unwrap()
            .with_tenant("tenant-1")
            .unwrap();
        let parts = IdempotencyScope::from_parts(Some("tenant-1"), &provider, "capture").unwrap();
        let unscoped = IdempotencyScope::from_parts(None, &provider, "capture").unwrap();
        assert_eq!(via_builder, parts);
        assert_eq!(
            (via_builder.as_str(), via_builder.tenant_id()),
            ("tenant-1:paymob:capture", Some("tenant-1"))
        );
        assert_eq!(
            (unscoped.as_str(), unscoped.tenant_id()),
            ("paymob:capture", None)
        );
    }

    #[test]
    fn scope_isolates_cross_tenant_keys_tm1() {
        let provider = ProviderId::new("stripe");
        let tenant_a = IdempotencyScope::from_parts(Some("tenant-a"), &provider, "refund").unwrap();
        let tenant_b = IdempotencyScope::from_parts(Some("tenant-b"), &provider, "refund").unwrap();
        let unscoped = IdempotencyScope::try_new(&provider, "refund").unwrap();
        assert_ne!(tenant_a.as_str(), tenant_b.as_str());
        assert_ne!(tenant_a.as_str(), unscoped.as_str());
        assert_ne!(tenant_b.as_str(), unscoped.as_str());
    }

    #[test]
    fn scope_rejects_invalid_operation_and_tenant_segments() {
        let provider = ProviderId::new("moyasar");
        let invalid_ops = build_invalid_segments("refund", MAX_SCOPE_SEGMENT_LEN, true);
        assert_all_rejected(&invalid_ops, |op| IdempotencyScope::try_new(&provider, op));

        let invalid_tenants = build_invalid_segments("tenant", MAX_SCOPE_SEGMENT_LEN, true);
        assert_all_rejected(&invalid_tenants, |tenant| {
            IdempotencyScope::try_new(&provider, "refund")
                .unwrap()
                .with_tenant(tenant)
        });
    }

    #[test]
    fn scope_accepts_max_segment_length_boundary() {
        let provider = ProviderId::new("stripe");
        let max_tenant = "t".repeat(MAX_SCOPE_SEGMENT_LEN);
        let max_op = "o".repeat(MAX_SCOPE_SEGMENT_LEN);
        let scope = IdempotencyScope::try_new(&provider, &max_op)
            .unwrap()
            .with_tenant(&max_tenant)
            .unwrap();
        assert_eq!(scope.tenant_id(), Some(max_tenant.as_str()));
        assert_eq!(scope.operation(), max_op.as_str());
    }

    #[test]
    fn uuid_v4_accepts_valid_and_rejects_invalid_formats() {
        let valid_uuids = [
            "550e8400-e29b-41d4-8716-446655440000",
            "550e8400-e29b-41d4-9716-446655440000",
            "550e8400-e29b-41d4-a716-446655440000",
            "550e8400-e29b-41d4-b716-446655440000",
            "550E8400-E29B-41D4-A716-446655440000",
            "01234567-89ab-4cde-8f01-23456789ABCD",
        ];
        for uuid in valid_uuids {
            assert!(validate_uuid_v4(uuid).is_ok());
        }
        let invalid_uuids: Vec<String> = [
            "550e8400-e29b-11d4-a716-446655440000",
            "550e8400-e29b-31d4-a716-446655440000",
            "550e8400-e29b-51d4-a716-446655440000",
            "550e8400-e29b-71d4-a716-446655440000",
            "550e8400-e29b-41d4-0716-446655440000",
            "550e8400-e29b-41d4-7716-446655440000",
            "550e8400-e29b-41d4-c716-446655440000",
            "550E8400-E29B-41D4-C716-446655440000",
            "",
            "550e8400-e29b-41d4-a716-44665544000",
            "550e8400-e29b-41d4-a716-4466554400000",
            "550e8400e29b41d4a716446655440000",
            "550e8400_e29b_41d4_a716_446655440000",
            "550e8400-e29b-41d4-a716-44665544000g",
            "550e840-0e29b-41d4-a716-446655440000",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_all_rejected(&invalid_uuids, validate_uuid_v4);
    }

    #[test]
    fn idempotency_key_accepts_valid_and_rejects_invalid_keys() {
        let max_key = "k".repeat(MAX_IDEMPOTENCY_KEY_LEN);
        for key in [
            "k",
            "refund:ord-1",
            "550e8400-e29b-41d4-a716-446655440000",
            &max_key,
        ] {
            assert!(validate_idempotency_key(key).is_ok());
        }
        let invalid_keys = build_invalid_segments("key", MAX_IDEMPOTENCY_KEY_LEN, false);
        assert_all_rejected(&invalid_keys, validate_idempotency_key);
    }

    #[test]
    fn validation_errors_never_leak_input_in_display_or_debug() {
        let provider = ProviderId::new("stripe");
        let secret_like = "sk_live_secret_value:bad";
        let scope_err = IdempotencyScope::try_new(&provider, secret_like).unwrap_err();
        let uuid_err = validate_uuid_v4(secret_like).unwrap_err();
        let key_err = validate_idempotency_key("sk_live_secret value").unwrap_err();
        for err in [scope_err, uuid_err, key_err] {
            assert!(!err.to_string().contains("sk_live_secret"));
            assert!(!format!("{err:?}").contains("sk_live_secret"));
        }
    }
}
