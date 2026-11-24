//! Consolidated human-path workflow integration test suite (`WF-01`..`WF-65` + import/export).
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;
#[path = "../human_wf_common/mod.rs"]
mod human_wf_common;

mod human_wf_01_06;
mod human_wf_07_12;
mod human_wf_13_18;
mod human_wf_19_24;
mod human_wf_25_31;
mod human_wf_32_37;
mod human_wf_38_43;
mod human_wf_44_50;
mod human_wf_51_58;
mod human_wf_59_65;
mod human_wf_import_export;
