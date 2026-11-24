#[allow(dead_code)]
pub fn assert_approx(actual: f64, expected: f64, tol: f64) {
    curvebook_testkit::assert_close(actual, expected, tol, "value");
}
