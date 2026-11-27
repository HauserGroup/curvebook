#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod archive_and_csv;
mod audit_p3_sec04;
mod audit_r1_s4a;
mod audit_r1_s4a_x1_03;
mod audit_r1_s4b_doc_fmt;
mod audit_r1_s4b_format;
mod audit_r1_s4b_recovery;
mod audit_r2_s01_s06;
mod audit_w1_sec04_s1;
mod audit_wave1_format_io;
mod codec_xml_zip;
mod corrupt_archive_fuzz;
mod external_archive_interop;
mod interop_and_export;
mod issue_59_pzfx_decimal_comma;
mod prop_format;
mod pzfx_full_roundtrip;
mod pzfx_interop_defects;
mod r13_7_image_assets;
mod r17_1_xlsx_and_csv;
mod r17_5_legacy_import_hardening;
mod r17_6_xlsx_uncached_formulas;
mod r17_7_import_and_schema_v8;
mod recovery_and_interop_parity;
mod rv_ledger_format;
mod schemas_doc_parity;
mod unpacked_and_pzfx_budget;
mod versioned_and_seams;
