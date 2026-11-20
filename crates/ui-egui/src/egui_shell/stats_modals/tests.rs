use super::model_formula::build_live_symbolic_formula;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, DataSheet, FactorialAnovaSpec, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype, VariableRole,
};

fn make_mv_doc(titles: &[&str]) -> (ProjectDocument, SheetId) {
    let mut doc = ProjectDocument::new("Formula Test").unwrap();
    let mv_id = SheetId::try_new("mv-1").unwrap();
    let mut mv = MultipleVariablesTable::new(titles.len(), 2).unwrap();
    for (i, t) in titles.iter().enumerate() {
        mv.variables[i].title = (*t).to_owned();
        mv.variables[i].role = if i == 0 { VariableRole::Continuous } else { VariableRole::Categorical };
    }
    let ds = DataSheet::new(mv_id.clone(), "MV", TableArchetype::MultipleVariables(mv)).unwrap();
    doc.data_sheets.push(ds);
    (doc, mv_id)
}

fn factorial_formula(doc: &ProjectDocument, mv_id: &SheetId, factors: Vec<usize>, order: usize) -> String {
    let fac = FactorialAnovaSpec { response_column: Some(0), multifactor_columns: factors, max_interaction_order: order, ..Default::default() };
    let spec = AnalysisSpec::new("res-fac", mv_id.clone(), AnalysisKind::FactorialAnova(fac)).unwrap();
    build_live_symbolic_formula(&spec, doc).unwrap()
}

#[test]
fn live_formula_generates_all_k_subsets_in_lexicographic_order() {
    let (doc, mv_id) = make_mv_doc(&["Y", "A", "B", "C", "D"]);

    assert_eq!(factorial_formula(&doc, &mv_id, vec![1, 2, 3], 1), "Y ~ A + B + C");
    assert_eq!(factorial_formula(&doc, &mv_id, vec![1, 2, 3], 2), "Y ~ A + B + C + A:B + A:C + B:C");
    assert_eq!(factorial_formula(&doc, &mv_id, vec![1, 2, 3], 3), "Y ~ A + B + C + A:B + A:C + B:C + A:B:C");
    assert_eq!(factorial_formula(&doc, &mv_id, vec![1, 2, 3, 4], 2), "Y ~ A + B + C + D + A:B + A:C + A:D + B:C + B:D + C:D");
    assert_eq!(
        factorial_formula(&doc, &mv_id, vec![1, 2, 3, 4], 3),
        "Y ~ A + B + C + D + A:B + A:C + A:D + B:C + B:D + C:D + A:B:C + A:B:D + A:C:D + B:C:D"
    );
    assert_eq!(
        factorial_formula(&doc, &mv_id, vec![1, 2, 3, 4], 5),
        "Y ~ A + B + C + D + A:B + A:C + A:D + B:C + B:D + C:D + A:B:C + A:B:D + A:C:D + B:C:D + A:B:C:D"
    );
}
