# Non-NIST fitting oracle scope

Follow-up to [#28](https://github.com/HauserGroup/curvebook/issues/28). This draft proposes D1 verification for built-in CurveBook equations outside the NIST nonlinear benchmark catalog. Existing coverage varies by model and by reported result.

## Proposed scope

Map the non-NIST catalog by model and verified output. Distinguish direct comparisons against independent values from solver recovery, finite-difference checks against the production evaluator, and catalog smoke tests. Use the inventory to select 8–10 new cases across curve and pharmacology families. Start with underrepresented peak, segmented, binding, or enzyme models as the inventory warrants, and include noisy weighted and constrained fits.

Use published equations for expected predictions and a versioned independent solver or closed-form calculation for fitted results. Match parameterization, weight convention, constraints, and the definition of reported uncertainty. Record the synthetic data and perturbations so a second implementation can reproduce the inputs.

## Acceptance criteria

- [ ] List each selected model and the exact outputs its reference verifies. Distinguish equation evaluation from solver and reporting checks.
- [ ] Assert independent predictions and, for cases that fit a model, fitted parameters and residual sums of squares. Assert the weighted objective explicitly for weighted cases.
- [ ] Include at least one noisy weighted fit and one constrained fit. Assert standard errors or confidence bounds where the configured method supports and reports them.
- [ ] Exercise representative end-to-end result sheets and verify that the reported fit quantities match the reference under the chosen options.
- [ ] Record source or generator version, inputs, expected outputs, and justified tolerances. Exercise the fixtures in offline Rust CI with existing assertions and tolerances intact.
