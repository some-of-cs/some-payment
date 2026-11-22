//! Integer minor-unit [`Money`] and ISO-4217 [`CurrencyCode`] primitives.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::PaymentError;

/// Minor-unit decimal scale of an ISO-4217 currency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MinorUnitScale {
    /// Zero-decimal currencies (e.g., `JPY`, `KRW`, `VND`).
    Zero,
    /// Standard two-decimal currencies (e.g., `USD`, `EUR`, `SAR`, `EGP`).
    Two,
    /// Three-decimal currencies (e.g., `KWD`, `BHD`, `JOD`, `OMR`, `TND`).
    Three,
}

const ZERO_DECIMAL_CURRENCIES: &[&str] = &[
    "BIF", "CLP", "DJF", "GNF", "JPY", "KMF", "KRW", "MGA", "PYG", "RWF", "UGX", "VND", "VUV",
    "XAF", "XOF", "XPF",
];

const THREE_DECIMAL_CURRENCIES: &[&str] = &["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"];

/// Validated uppercase 3-letter ISO-4217 currency code.
///
/// # Examples
///
/// ```
/// use some_payment_core::{CurrencyCode, MinorUnitScale};
///
/// let sar = CurrencyCode::try_new("sar").unwrap();
/// assert_eq!(sar.as_str(), "SAR");
/// assert_eq!(sar.scale(), MinorUnitScale::Two);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    /// Validates and normalizes a 3-letter ASCII ISO-4217 currency code.
    pub fn try_new(code: impl AsRef<str>) -> Result<Self, PaymentError> {
        let normalized = code.as_ref().trim().to_ascii_uppercase();
        if normalized.len() != 3 || !normalized.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(PaymentError::validation(
                "currency must be a 3-letter ISO-4217 ASCII code",
            ));
        }
        Ok(Self(normalized))
    }

    /// Returns the uppercase ISO-4217 code (e.g., `"SAR"`).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the lowercase ISO-4217 code (e.g., `"sar"` for Stripe wire payloads).
    pub fn to_lowercase(&self) -> String {
        self.0.to_ascii_lowercase()
    }

    /// Returns the standard ISO-4217 minor-unit decimal scale for this currency.
    pub fn scale(&self) -> MinorUnitScale {
        if ZERO_DECIMAL_CURRENCIES.contains(&self.0.as_str()) {
            MinorUnitScale::Zero
        } else if THREE_DECIMAL_CURRENCIES.contains(&self.0.as_str()) {
            MinorUnitScale::Three
        } else {
            MinorUnitScale::Two
        }
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CurrencyCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::try_new(raw).map_err(serde::de::Error::custom)
    }
}

/// Strictly positive integer minor-unit monetary value (never floating point).
///
/// # Examples
///
/// ```
/// use some_payment_core::Money;
///
/// let price = Money::new(15_000, "sar").unwrap();
/// assert_eq!(price.amount(), 15_000);
/// assert_eq!(price.currency().as_str(), "SAR");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Money {
    amount: u64,
    currency: CurrencyCode,
}

impl Money {
    /// Constructs a validated positive minor-unit [`Money`] value.
    pub fn new(amount: u64, currency: impl AsRef<str>) -> Result<Self, PaymentError> {
        if amount == 0 {
            return Err(PaymentError::validation(
                "monetary amount must be strictly greater than zero",
            ));
        }
        let currency = CurrencyCode::try_new(currency)?;
        Ok(Self { amount, currency })
    }

    /// Returns the integer minor-unit amount.
    pub fn amount(&self) -> u64 {
        self.amount
    }

    /// Returns the validated ISO-4217 [`CurrencyCode`].
    pub fn currency(&self) -> &CurrencyCode {
        &self.currency
    }

    /// Checked addition requiring identical currencies.
    pub fn checked_add(&self, rhs: &Self) -> Result<Self, PaymentError> {
        if self.currency != rhs.currency {
            return Err(PaymentError::validation(
                "cannot add Money values with different currencies",
            ));
        }
        let sum = self
            .amount
            .checked_add(rhs.amount)
            .ok_or_else(|| PaymentError::validation("monetary addition overflow"))?;
        Ok(Self {
            amount: sum,
            currency: self.currency.clone(),
        })
    }

    /// Checked subtraction requiring identical currencies and a strictly positive remainder.
    pub fn checked_sub(&self, rhs: &Self) -> Result<Self, PaymentError> {
        if self.currency != rhs.currency {
            return Err(PaymentError::validation(
                "cannot subtract Money values with different currencies",
            ));
        }
        let diff = self
            .amount
            .checked_sub(rhs.amount)
            .ok_or_else(|| PaymentError::validation("monetary subtraction underflow"))?;
        if diff == 0 {
            return Err(PaymentError::validation(
                "monetary subtraction resulted in zero",
            ));
        }
        Ok(Self {
            amount: diff,
            currency: self.currency.clone(),
        })
    }
}

#[derive(Deserialize)]
struct RawMoney {
    amount: u64,
    currency: String,
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawMoney::deserialize(deserializer)?;
        Self::new(raw.amount, raw.currency).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_and_classifies_scales() {
        let sar = Money::new(15_000, "sar").unwrap();
        assert_eq!(sar.amount(), 15_000);
        assert_eq!(sar.currency().as_str(), "SAR");
        assert_eq!(sar.currency().to_lowercase(), "sar");
        assert_eq!(sar.currency().to_string(), "SAR");
        assert_eq!(sar.currency().scale(), MinorUnitScale::Two);

        let jpy = Money::new(500, "JPY").unwrap();
        assert_eq!(jpy.currency().scale(), MinorUnitScale::Zero);

        let kwd = Money::new(1_250, "kwd").unwrap();
        assert_eq!(kwd.currency().scale(), MinorUnitScale::Three);
    }

    #[test]
    fn checked_add_and_checked_sub_behavior() {
        let a = Money::new(1_000, "USD").unwrap();
        let b = Money::new(400, "USD").unwrap();
        let eur = Money::new(400, "EUR").unwrap();

        let sum = a.checked_add(&b).unwrap();
        assert_eq!(sum.amount(), 1_400);
        assert_eq!(sum.currency().as_str(), "USD");

        let diff = a.checked_sub(&b).unwrap();
        assert_eq!(diff.amount(), 600);
        assert_eq!(diff.currency().as_str(), "USD");

        assert!(a.checked_sub(&a).is_err());
        assert!(b.checked_sub(&a).is_err());
        assert!(a.checked_sub(&eur).is_err());
    }

    #[test]
    fn rejects_zero_invalid_currency_and_overflow() {
        assert!(Money::new(0, "USD").is_err());
        assert!(Money::new(100, "US").is_err());
        assert!(Money::new(100, "123").is_err());

        let max = Money::new(u64::MAX, "USD").unwrap();
        let one = Money::new(1, "USD").unwrap();
        assert!(max.checked_add(&one).is_err());

        let eur = Money::new(100, "EUR").unwrap();
        assert!(one.checked_add(&eur).is_err());
    }

    #[test]
    fn serde_rejects_zero_amount() {
        let bad: Result<Money, _> = serde_json::from_str(r#"{"amount":0,"currency":"USD"}"#);
        assert!(bad.is_err());
    }
}
