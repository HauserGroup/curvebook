//! Permanent regression gate for `R0.14`: SemVer pre-release versions (`-alpha.N`, `-beta.N`,
//! `-rc.N`, ...) must always be drafted as GitHub prereleases, so a release-candidate tag push can
//! never publish a candidate as the repository's "Latest" release.

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).ok_or_else(|| String::from("CARGO_MANIFEST_DIR has no parent"))
}

#[cfg(unix)]
fn classify(args: &[&str]) -> Result<std::process::Output, String> {
    std::process::Command::new("bash")
        .arg(workspace_root()?.join("packaging/release-channel.sh"))
        .args(args)
        .output()
        .map_err(|e| format!("run packaging/release-channel.sh: {e}"))
}

#[cfg(unix)]
#[test]
fn r014_release_channel_marks_semver_prereleases() {
    // (version, alpha input, expected prerelease, expected title)
    let cases: &[(&str, &str, &str, &str)] = &[
        ("0.1.0-alpha.1", "true", "true", "CurveBook Alpha v0.1.0-alpha.1"),
        ("0.1.0-alpha.2", "", "true", "CurveBook Alpha v0.1.0-alpha.2"),
        ("v0.1.0-rc.1", "", "true", "CurveBook Release Candidate v0.1.0-rc.1"),
        ("0.1.0-rc1", "", "true", "CurveBook Release Candidate v0.1.0-rc1"),
        ("0.2.0-BETA.2", "", "true", "CurveBook Beta v0.2.0-BETA.2"),
        ("1.0.0-dev.3+sha.abc", "", "true", "CurveBook Prerelease v1.0.0-dev.3+sha.abc"),
        ("1.0.0", "true", "true", "CurveBook Alpha v1.0.0"),
        ("1.0.0", "", "false", "CurveBook v1.0.0"),
        ("1.0.0", "false", "false", "CurveBook v1.0.0"),
        // A hyphen inside build metadata is not a pre-release marker.
        ("1.0.0+build-5", "", "false", "CurveBook v1.0.0+build-5"),
    ];
    for &(ver, alpha, prerelease, title) in cases {
        let out = classify(&[ver, alpha]).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{ver}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(stdout, format!("prerelease={prerelease}\ntitle={title}\n"), "{ver} (alpha={alpha:?})");
    }
}

#[cfg(unix)]
#[test]
fn r014_release_channel_rejects_invalid_versions() {
    for bad in ["", "0.1", "1.0.0-", "1.0.0+", "v", "1.0.0 rc", "1.0.0-rc.1\n"] {
        let out = classify(&[bad]).unwrap();
        assert!(!out.status.success(), "{bad:?} must be rejected");
        assert!(out.stdout.is_empty(), "{bad:?} must not emit outputs: {}", String::from_utf8_lossy(&out.stdout));
    }
}

#[test]
fn r014_release_workflow_uses_release_channel_for_prerelease_flag() {
    let release = fs::read_to_string(workspace_root().unwrap().join(".github/workflows/release.yml")).expect("read release.yml");
    assert!(
        release.contains("./packaging/release-channel.sh \"$ver\" \"${ALPHA_RELEASE:-}\" >> \"$GITHUB_OUTPUT\""),
        "the publish job must classify the resolved version with packaging/release-channel.sh"
    );
    assert!(release.contains("RELEASE_PRERELEASE: ${{ steps.release_meta.outputs.prerelease }}"));
    assert!(release.contains("RELEASE_TITLE: ${{ steps.release_meta.outputs.title }}"));
    assert!(release.contains("release_flags+=(--latest=false)"), "prereleases must never become the Latest release");
    assert!(release.contains("release channel did not classify"), "an unclassified version must fail closed");
    assert!(!release.contains("prerelease=false\n"), "the prerelease flag must not be hardcoded in the workflow");
}
