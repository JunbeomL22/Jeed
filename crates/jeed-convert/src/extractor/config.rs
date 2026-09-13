//! Configuration for fixed-width numeric extraction.

use crate::ConfigErr;

/// Configuration for fixed-width numeric extraction.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct Config {
    /// Total input length in bytes.
    pub total_size: usize,
    /// Index of decimal point (relative to numeric start).
    pub numeric_point_idx: Option<usize>,
    /// Number of digits in fractional part.
    pub fraction_size: usize,
    /// Number of digits in integer part (including unused_head).
    pub integer_size: usize,
    /// Whether number supports negative values.
    pub is_signed: bool,
    /// Leading bytes to skip during parsing.
    pub unused_head: usize,
    /// Trailing bytes to skip during parsing.
    pub unused_tail: usize,
    /// Length of actual numeric portion.
    pub used_numeric_length: usize,
    /// Multiplier for float conversion (1/10^fraction_size).
    pub fraction_multiplier: f64,
    /// Divisor for scaling (10^fraction_size).
    pub fraction_divisor: u64,
    /// Start index of numeric portion.
    pub numeric_start_idx: usize,
    /// End index of numeric portion.
    pub numeric_end_idx: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            total_size: 0,
            numeric_point_idx: None,
            fraction_size: 0,
            integer_size: 0,
            is_signed: false,
            unused_head: 0,
            unused_tail: 0,
            used_numeric_length: 0,
            fraction_multiplier: 0.0,
            fraction_divisor: 0,
            numeric_start_idx: 0,
            numeric_end_idx: 0,
        }
    }
}

impl Config {
    /// Creates a builder with default values.
    pub fn builder() -> Config {
        Config::default()
    }

    /// Sets the integer size.
    pub fn with_integer_size(mut self, integer_size: usize) -> Self {
        self.integer_size = integer_size;
        self
    }

    /// Sets the fraction size.
    pub fn with_fraction_size(mut self, fraction_size: usize) -> Self {
        self.fraction_size = fraction_size;
        self
    }

    /// Sets whether the number is signed.
    pub fn with_is_signed(mut self, is_signed: bool) -> Self {
        self.is_signed = is_signed;
        self
    }

    /// Sets the unused head size.
    pub fn with_unused_head(mut self, unused_head: usize) -> Self {
        self.unused_head = unused_head;
        self
    }

    /// Sets the unused tail size.
    pub fn with_unused_tail(mut self, unused_tail: usize) -> Self {
        self.unused_tail = unused_tail;
        self
    }

    /// Sets the numeric point index manually.
    pub fn with_numeric_point_idx(mut self, numeric_point_idx: Option<usize>) -> Self {
        self.numeric_point_idx = numeric_point_idx;
        self
    }

    /// Builds and validates the configuration.
    ///
    /// # Errors
    /// Returns [`ConfigErr::InvalidSize`] if configuration is invalid.
    pub fn build(&mut self) -> Result<&mut Self, ConfigErr> {
        self.compute_total_size();
        self.compute_numeric_indices();
        self.compute_numeric_point_idx();
        self.compute_used_numeric_length();
        self.validate()?;
        self.compute_fraction_factors();
        Ok(self)
    }

    fn compute_total_size(&mut self) {
        let sign_size = if self.is_signed { 1 } else { 0 };
        let fraction_total = if self.fraction_size > 0 {
            1 + self.fraction_size
        } else {
            0
        };
        self.total_size = self.integer_size + sign_size + fraction_total;
    }

    fn compute_numeric_indices(&mut self) {
        self.numeric_start_idx = self.unused_head;
    }

    fn compute_numeric_point_idx(&mut self) {
        if self.fraction_size == 0 {
            self.numeric_point_idx = None;
            return;
        }

        let effective_fraction_size = self.fraction_size as i64 - self.unused_tail as i64;
        if effective_fraction_size <= 0 {
            self.numeric_point_idx = None;
        } else {
            self.numeric_point_idx = Some(self.integer_size - self.numeric_start_idx);
        }
    }

    fn compute_used_numeric_length(&mut self) {
        let fraction_total = if self.fraction_size > 0 {
            1 + self.fraction_size
        } else {
            0
        };
        self.used_numeric_length =
            self.integer_size + fraction_total - self.unused_head - self.unused_tail;
        self.numeric_end_idx = self.numeric_start_idx + self.used_numeric_length;
    }

    fn validate(&self) -> Result<(), ConfigErr> {
        let min_required = if self.is_signed { 1 } else { 0 };
        if self.total_size < min_required {
            return Err(ConfigErr::InvalidSize);
        }

        if self.integer_size == 0 && self.fraction_size == 0 {
            return Err(ConfigErr::InvalidSize);
        }

        let numeric_digits = self.integer_size + self.fraction_size;
        if self.unused_head + self.unused_tail > numeric_digits {
            return Err(ConfigErr::InvalidSize);
        }

        Ok(())
    }

    fn compute_fraction_factors(&mut self) {
        let exponent = self.fraction_size.saturating_sub(self.unused_tail);
        self.fraction_divisor = 10u64.pow(exponent as u32);
        self.fraction_multiplier = 1.0 / self.fraction_divisor as f64;
    }
}
