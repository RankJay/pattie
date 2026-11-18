//! The Bessel function both resamplers use for their Kaiser window.

/// The modified Bessel function of the first kind, order zero, by its series.
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > sum * 1e-12 {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::bessel_i0;

    /// The resampler's Kaiser window: I0 against its published values
    /// (Abramowitz and Stegun, Table 9.8; checked against mpmath's `besseli` at 30 digits).
    #[test]
    fn bessel_i0_matches_its_published_values() {
        for (x, i0) in [
            (0.0, 1.0),
            (1.0, 1.266_065_877_752_008_4),
            (5.0, 27.239_871_823_604_442),
            (10.0, 2_815.716_628_466_254),
        ] {
            let got = bessel_i0(x);
            assert!((got - i0).abs() <= i0 * 1e-12, "I0({x}) = {got}, not {i0}");
        }
    }
}
