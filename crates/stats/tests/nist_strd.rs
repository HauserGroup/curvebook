//! NIST StRD certification integration tests for `curvebook-stats` across all 31 datasets:
//! 9 Univariate, 11 One-Way ANOVA, and 11 Linear Regression datasets.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use curvebook_math::{ConfidenceLevel, DenseMatrix};
use curvebook_stats::anova::ordinary_one_way_anova;
use curvebook_stats::column::summarize_column;
use curvebook_stats::regression::multiple_linear_regression;
use curvebook_testkit::nist_strd::{
    all_offline_anova_benchmarks, all_offline_linear_regression_benchmarks, all_offline_univariate_benchmarks, corpus_available,
    default_nist_strd_dir, load_corpus_anova_all, load_corpus_linear_all, load_corpus_univariate_all,
};
use curvebook_testkit::{assert_approx_eq, assert_lre_at_least, log_relative_error as lre, oracle_univariate_f64};

fn require_corpus_dir() -> Option<std::path::PathBuf> {
    let dir = default_nist_strd_dir();
    if corpus_available(&dir) {
        return Some(dir);
    }
    assert!(
        std::env::var_os("CURVEBOOK_REQUIRE_CORPUS").is_none_or(|v| v != "1"),
        "CURVEBOOK_REQUIRE_CORPUS=1 set, but NIST StRD corpus is missing or incomplete at {}",
        dir.display()
    );
    None
}

#[test]
fn nist_strd_offline_benchmarks_univariate_anova_linear() {
    for ds in all_offline_univariate_benchmarks() {
        let summary = summarize_column(&ds.name, &ds.values, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
        let (o_mean, o_sd, o_r1) = oracle_univariate_f64(&ds.values);
        assert_lre_at_least(summary.mean, o_mean, 14.5, &format!("{} offline mean (oracle)", ds.name));
        assert_lre_at_least(summary.sd.unwrap(), o_sd, 14.5, &format!("{} offline sd (oracle)", ds.name));
        assert_approx_eq(summary.lag1_autocorrelation.unwrap(), o_r1, 1e-14, 1e-13, &format!("{} offline r1", ds.name));
    }

    for ds in all_offline_anova_benchmarks() {
        let mut grouped: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
        for &(g, val) in &ds.observations {
            grouped.entry(g).or_default().push(val);
        }
        let labels: Vec<(String, Vec<f64>)> = grouped.into_iter().map(|(g, v)| (format!("G{g}"), v)).collect();
        let refs: Vec<(&str, &[f64])> = labels.iter().map(|(k, v)| (k.as_str(), v.as_slice())).collect();
        let res = ordinary_one_way_anova(&refs, ConfidenceLevel::DEFAULT_95).unwrap();
        assert_eq!(res.df_between as usize, ds.certified_df_between);
        assert_eq!(res.df_within as usize, ds.certified_df_within);
        let floor = if ds.name.starts_with("SmLs07") || ds.name.starts_with("SmLs08") || ds.name.starts_with("SmLs09") { 3.0 } else { 13.5 };
        assert_lre_at_least(res.f_statistic, ds.certified_f, floor, &format!("{} offline F", ds.name));
    }

    for ds in all_offline_linear_regression_benchmarks() {
        let n = ds.design_rows.len();
        let p = ds.certified_betas.len();
        let mut flat = Vec::with_capacity(n * p);
        for row in &ds.design_rows {
            flat.extend_from_slice(row);
        }
        let x = DenseMatrix::from_row_major(n, p, flat).unwrap();
        let term_names: Vec<String> = (0..p).map(|j| format!("B{j}")).collect();
        let has_intercept = !ds.name.starts_with("NoInt");
        let fit = multiple_linear_regression(&x, &ds.y, &term_names, has_intercept, false, ConfidenceLevel::DEFAULT_95).unwrap();
        let floor = if ds.name == "Wampler2" { 13.0 } else { 13.5 };
        for (j, (term, &cert_b)) in fit.terms.iter().zip(ds.certified_betas.iter()).enumerate() {
            assert_lre_at_least(term.estimate, cert_b, floor, &format!("{} offline B{j}", ds.name));
        }
    }
}

#[test]
fn nist_strd_real_corpus_univariate_all_9_datasets() {
    let Some(dir) = require_corpus_dir() else {
        return;
    };
    let datasets = load_corpus_univariate_all(&dir).unwrap();
    assert_eq!(datasets.len(), 9);

    for ds in datasets {
        let summary = summarize_column(&ds.name, &ds.values, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
        let sd = summary.sd.unwrap();
        let r1 = summary.lag1_autocorrelation.unwrap();

        let (oracle_mean, oracle_sd, oracle_r1) = oracle_univariate_f64(&ds.values);
        assert_lre_at_least(summary.mean, oracle_mean, 14.5, &format!("{} mean (f64 oracle)", ds.name));
        assert_lre_at_least(sd, oracle_sd, 14.5, &format!("{} sd (f64 oracle)", ds.name));
        let oracle_r1_tol = match ds.name.as_str() {
            "NumAcc3" | "NumAcc4" => 1e-10,
            _ => 1e-13,
        };
        assert_approx_eq(r1, oracle_r1, 1e-14, oracle_r1_tol, &format!("{} r1 (f64 oracle)", ds.name));

        assert_lre_at_least(summary.mean, ds.certified_mean, 14.5, &format!("{} decimal mean", ds.name));
        let (min_sd_lre, min_r1_lre) = match ds.name.as_str() {
            "NumAcc3" => (9.2, 11.5),
            "NumAcc4" => (8.0, 10.5),
            "Mavro" => (13.0, 13.5),
            "Michelso" => (13.5, 13.2),
            _ => (15.0, 14.5),
        };
        assert_lre_at_least(sd, ds.certified_sd, min_sd_lre, &format!("{} decimal sd", ds.name));
        assert_lre_at_least(r1, ds.certified_r1, min_r1_lre, &format!("{} decimal r1", ds.name));

        let lre_mean = lre(summary.mean, ds.certified_mean);
        let lre_sd = lre(sd, ds.certified_sd);
        let lre_r1 = lre(r1, ds.certified_r1);
        println!(
            "[UNIV] {:<10} mean_LRE={:.2} sd_LRE={:.2} r1_LRE={:.2} (oracle_sd_LRE={:.2})",
            ds.name,
            lre_mean,
            lre_sd,
            lre_r1,
            lre(sd, oracle_sd)
        );
    }
}

#[test]
fn nist_strd_real_corpus_anova_all_11_datasets() {
    let Some(dir) = require_corpus_dir() else {
        return;
    };
    let datasets = load_corpus_anova_all(&dir).unwrap();
    assert_eq!(datasets.len(), 11);

    for ds in datasets {
        let mut grouped: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
        for &(g, val) in &ds.observations {
            grouped.entry(g).or_default().push(val);
        }
        let labels: Vec<(String, Vec<f64>)> = grouped.into_iter().map(|(g, v)| (format!("Group_{g}"), v)).collect();
        let refs: Vec<(&str, &[f64])> = labels.iter().map(|(k, v)| (k.as_str(), v.as_slice())).collect();

        let res = ordinary_one_way_anova(&refs, ConfidenceLevel::DEFAULT_95).unwrap();
        assert_eq!(res.df_between as usize, ds.certified_df_between, "{} df_between", ds.name);
        assert_eq!(res.df_within as usize, ds.certified_df_within, "{} df_within", ds.name);

        let min_lre = match ds.name.as_str() {
            "SmLs07" | "SmLs08" | "SmLs09" => 3.0,
            "AtmWtAg" => 8.5,
            "SmLs04" | "SmLs05" | "SmLs06" => 9.0,
            "SiRstv" => 13.0,
            _ => 14.5,
        };
        let rsd = res.ms_within.sqrt();
        assert_lre_at_least(res.ss_between, ds.certified_ss_between, min_lre, &format!("{} SS_between", ds.name));
        assert_lre_at_least(res.ss_within, ds.certified_ss_within, min_lre, &format!("{} SS_within", ds.name));
        assert_lre_at_least(res.f_statistic, ds.certified_f, min_lre, &format!("{} F", ds.name));
        assert_lre_at_least(res.r_squared, ds.certified_r_squared, min_lre, &format!("{} R^2", ds.name));
        assert_lre_at_least(rsd, ds.certified_residual_sd, min_lre, &format!("{} RSD", ds.name));

        let min_obs_lre = [
            lre(res.ss_between, ds.certified_ss_between),
            lre(res.ss_within, ds.certified_ss_within),
            lre(res.f_statistic, ds.certified_f),
            lre(res.r_squared, ds.certified_r_squared),
            lre(rsd, ds.certified_residual_sd),
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
        println!(
            "[ANOVA] {:<10} F_LRE={:.2} SSb_LRE={:.2} SSw_LRE={:.2} RSD_LRE={:.2} min_LRE={:.2}",
            ds.name,
            lre(res.f_statistic, ds.certified_f),
            lre(res.ss_between, ds.certified_ss_between),
            lre(res.ss_within, ds.certified_ss_within),
            lre(rsd, ds.certified_residual_sd),
            min_obs_lre
        );
    }
}

#[test]
fn nist_strd_real_corpus_linear_regression_all_11_datasets() {
    let Some(dir) = require_corpus_dir() else {
        return;
    };
    let datasets = load_corpus_linear_all(&dir).unwrap();
    assert_eq!(datasets.len(), 11);

    for ds in datasets {
        let n = ds.design_rows.len();
        let p = ds.certified_betas.len();
        let mut flat = Vec::with_capacity(n * p);
        for row in &ds.design_rows {
            flat.extend_from_slice(row);
        }
        let x = DenseMatrix::from_row_major(n, p, flat).unwrap();
        let has_intercept = !ds.name.starts_with("NoInt");
        let term_names: Vec<String> = (0..p).map(|j| if has_intercept && j == 0 { String::from("Intercept") } else { format!("B{j}") }).collect();

        let fit = multiple_linear_regression(&x, &ds.y, &term_names, has_intercept, false, ConfidenceLevel::DEFAULT_95).unwrap();
        assert_eq!(fit.rank, p, "{} expected full rank {p}, got {}", ds.name, fit.rank);

        let beta_floor = match ds.name.as_str() {
            "Filip" => 7.5,
            "Wampler2" => 13.0,
            _ => 13.5,
        };
        let mut min_beta_lre = f64::INFINITY;
        for (j, (term, &cert_b)) in fit.terms.iter().zip(ds.certified_betas.iter()).enumerate() {
            assert_lre_at_least(term.estimate, cert_b, beta_floor, &format!("{} B{j}", ds.name));
            min_beta_lre = min_beta_lre.min(lre(term.estimate, cert_b));
        }

        let mut min_se_lre = f64::INFINITY;
        let rss_lre = if ds.certified_ss_res > 0.0 {
            let rss_floor = match ds.name.as_str() {
                "Filip" => 8.5,
                _ => 13.5,
            };
            assert_lre_at_least(fit.rss, ds.certified_ss_res, rss_floor, &format!("{} RSS", ds.name));
            assert_lre_at_least(fit.residual_sd, ds.certified_residual_sd, rss_floor, &format!("{} RSD", ds.name));
            let se_floor = match ds.name.as_str() {
                "Filip" => 7.2,
                "Longley" => 12.5,
                _ => 13.5,
            };
            for (j, (term, &cert_se)) in fit.terms.iter().zip(ds.certified_se_betas.iter()).enumerate() {
                assert_lre_at_least(term.se, cert_se, se_floor, &format!("{} SE(B{j})", ds.name));
                min_se_lre = min_se_lre.min(lre(term.se, cert_se));
            }
            lre(fit.rss, ds.certified_ss_res)
        } else {
            assert!(fit.rss < 1e-14, "{} exact polynomial RSS={:.3e}", ds.name, fit.rss);
            min_se_lre = 15.0;
            15.0
        };

        println!("[LLS]   {:<10} min_beta_LRE={:.2} min_SE_LRE={:.2} RSS_LRE={:.2}", ds.name, min_beta_lre, min_se_lre, rss_lre);
    }
}
