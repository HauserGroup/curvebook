//! Pinned SHA-256 + byte-size catalog and verified on-disk loaders for all 58 NIST StRD `.dat` files.
//!
//! Governing spec: `docs/development.md` (§Verified NIST StRD corpus). Every loader verifies the
//! exact byte count and SHA-256 digest via [`load_verified_corpus_file`] before parsing.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::nist_strd::parser::{
    AnovaDataset, LinearRegressionDataset, NonlinearDataset, StrdParseError, UnivariateDataset, parse_anova_dat, parse_linear_regression_dat,
    parse_nonlinear_dat, parse_univariate_dat,
};
use crate::sha256::{CorpusError, CorpusManifestEntry, load_verified_corpus_file};

/// NIST StRD reference suite category for the 58-file corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrdCorpusSuite {
    Univariate,
    Anova,
    LinearRegression,
    NonlinearRegression,
}

impl StrdCorpusSuite {
    /// Subdirectory under `corpus/nist-strd/` containing this suite's `.dat` files.
    #[must_use]
    pub const fn subdir(self) -> &'static str {
        match self {
            Self::Univariate => "univ",
            Self::Anova => "anova",
            Self::LinearRegression => "lls",
            Self::NonlinearRegression => "nls",
        }
    }
}

/// Pinned NIST StRD dataset specification with suite metadata and SHA-256 manifest entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedStrdSpec {
    pub name: &'static str,
    pub suite: StrdCorpusSuite,
    pub manifest: CorpusManifestEntry<'static>,
}

const fn pinned(
    name: &'static str,
    suite: StrdCorpusSuite,
    relative_path: &'static str,
    expected_bytes: usize,
    sha256_hex: &'static str,
) -> PinnedStrdSpec {
    PinnedStrdSpec { name, suite, manifest: CorpusManifestEntry { relative_path, expected_bytes, sha256_hex } }
}

/// Pinned SHA-256 digests and exact byte sizes for all 58 public-domain NIST StRD `.dat` files
/// matching `corpus/nist-strd/sha256sums.txt` (`9` univariate, `11` ANOVA, `11` linear, `27` nonlinear).
pub const PINNED_NIST_STRD_CORPUS: &[PinnedStrdSpec] = &[
    // Univariate summary statistics (9)
    pinned("PiDigits", StrdCorpusSuite::Univariate, "univ/PiDigits.dat", 46_441, "e9421a2f4c0a31c505099332b4d22e507e808953c3c57bddae9226f25587d7d2"),
    pinned("Lottery", StrdCorpusSuite::Univariate, "univ/Lottery.dat", 3_750, "956cb33392ee28683aa7978326fc356d1461e4a3b0e5a69d317fd481d02d6255"),
    pinned("Lew", StrdCorpusSuite::Univariate, "univ/Lew.dat", 3_518, "081b231f686d01fa00247b8072909206947c8321bd62e060d9aaae5f4866c44e"),
    pinned("Mavro", StrdCorpusSuite::Univariate, "univ/Mavro.dat", 2_187, "6b967538d6a607fdf30df054c6fbf4ca6bfb4fa6dc2ecbe613d4f93237a0b1dd"),
    pinned("Michelso", StrdCorpusSuite::Univariate, "univ/Michelso.dat", 2_349, "68cad435641539ca020adafb4c9a6b7fcc4fefe1d6ab8f2af8fb858d4f1b12ab"),
    pinned("NumAcc1", StrdCorpusSuite::Univariate, "univ/NumAcc1.dat", 1_840, "ce7c7a0fc6d7cedd0d5e1f9911afb9337544be6836730beb9968ca0a4571d375"),
    pinned("NumAcc2", StrdCorpusSuite::Univariate, "univ/NumAcc2.dat", 11_794, "5be29522dfbe299541f0ec98ec1c22a51a5587bf06561ba21a92c209fb7ef0f9"),
    pinned("NumAcc3", StrdCorpusSuite::Univariate, "univ/NumAcc3.dat", 14_830, "66df1b96596735936f37fd889e7c1c6ce998eb2b523bfc46c98ec1737e992aec"),
    pinned("NumAcc4", StrdCorpusSuite::Univariate, "univ/NumAcc4.dat", 15_765, "ca310dc767f5130f980f8280bbe69e26ba414a4d85a89fc11b6c767b828e5fe4"),
    // One-way ANOVA (11)
    pinned("SiRstv", StrdCorpusSuite::Anova, "anova/SiRstv.dat", 2_032, "5a4f02d7f256e18c30dcad1b81e0260bb810b5ac113f6107e06dee168d07663e"),
    pinned("AtmWtAg", StrdCorpusSuite::Anova, "anova/AtmWtAg.dat", 3_171, "5a1083c9062d3966f244bffa376fc9b033244abf86a0d0dd016af39cd71fb8a7"),
    pinned("SmLs01", StrdCorpusSuite::Anova, "anova/SmLs01.dat", 6_304, "ad28cac0d4fb79ffea2522b55349083bdea10f12a6f8c46cfbabcccdb4d6b896"),
    pinned("SmLs02", StrdCorpusSuite::Anova, "anova/SmLs02.dat", 48_430, "708e32eaf36769abe7533e7dcd1b6dc67091b60df938dd37c4f0e65660ef7b0b"),
    pinned("SmLs03", StrdCorpusSuite::Anova, "anova/SmLs03.dat", 469_635, "e18404d0628b78670580b230a3a3b6cd997d959d00f92a2883a4da942b19377a"),
    pinned("SmLs04", StrdCorpusSuite::Anova, "anova/SmLs04.dat", 7_064, "1308eb1581f28ce6c9de4ca9520fcc729f916c34f1f9dee10fb6e6581e5b4ce7"),
    pinned("SmLs05", StrdCorpusSuite::Anova, "anova/SmLs05.dat", 55_668, "7b1333e6e56ced74145096df774cfb1f6305213cb28f9a8222c418ca85d0c296"),
    pinned("SmLs06", StrdCorpusSuite::Anova, "anova/SmLs06.dat", 541_674, "0c0f09a2df05f2e9db5324a6aaeed85647651e874e9f5c3892b8f29fc4a200f7"),
    pinned("SmLs07", StrdCorpusSuite::Anova, "anova/SmLs07.dat", 7_630, "fd95c510aa8b3ce7f67ddd32846ccead7d9abae441790336c6fb0fcb0ae3558f"),
    pinned("SmLs08", StrdCorpusSuite::Anova, "anova/SmLs08.dat", 61_097, "87dd38dfffeb65d39dbe3e8ba73bd16644fc396c1aad03fc0295225f3ba6aef9"),
    pinned("SmLs09", StrdCorpusSuite::Anova, "anova/SmLs09.dat", 595_702, "f4688c5a0d5690fc86fa27e0c5a265eef0b24641e74af27aa72d9615084dd13f"),
    // Linear least-squares regression (11)
    pinned("Norris", StrdCorpusSuite::LinearRegression, "lls/Norris.dat", 2_688, "5ab6906c68a9c4b2eb1661c0cd7d2a09f5a469a6d4e05b6b691f190f5f7b944f"),
    pinned(
        "Pontius",
        StrdCorpusSuite::LinearRegression,
        "lls/Pontius.dat",
        2_748,
        "de60baa5dc66fd804f9316f895bc5aa2b8f791c09d1ecc1ec39f8b199b8c15c7",
    ),
    pinned("NoInt1", StrdCorpusSuite::LinearRegression, "lls/NoInt1.dat", 1_608, "d8234c428dedaa9de23b67df7d04aeee83befae73ceb6b38321b224955a9b667"),
    pinned("NoInt2", StrdCorpusSuite::LinearRegression, "lls/NoInt2.dat", 1_347, "a71c2810b1edcef8fec6b864c20c5f9d9908e530282a174772ce610999ca7c69"),
    pinned("Filip", StrdCorpusSuite::LinearRegression, "lls/Filip.dat", 4_867, "403b34689e401d915cb28bd69311d3ca5c3007d88d3b670b00e3203042315072"),
    pinned(
        "Longley",
        StrdCorpusSuite::LinearRegression,
        "lls/Longley.dat",
        2_987,
        "fc4b0c824f8f0b834eb8e7d1c4c7e0b3cebdd7987bcb085e8f5e1007582480ea",
    ),
    pinned(
        "Wampler1",
        StrdCorpusSuite::LinearRegression,
        "lls/Wampler1.dat",
        2_420,
        "8bcd6f00dbe8c307ebc5c971e12b950eb3cdb75afbc2b38573082b5fcb0d7fbc",
    ),
    pinned(
        "Wampler2",
        StrdCorpusSuite::LinearRegression,
        "lls/Wampler2.dat",
        2_414,
        "01ba5f08b5b9f84431f3754216e006939a80264a1caa69fe08b3cbc543f9ee3e",
    ),
    pinned(
        "Wampler3",
        StrdCorpusSuite::LinearRegression,
        "lls/Wampler3.dat",
        2_433,
        "c5b025f6ebf0365de92d80e478ad826b01a6dfdafb71964ea8bae7eec993ce53",
    ),
    pinned(
        "Wampler4",
        StrdCorpusSuite::LinearRegression,
        "lls/Wampler4.dat",
        2_381,
        "dcff32075a52ecf31af70dad340776237544b1a29b5034a40f8b3e6b607cf9c1",
    ),
    pinned(
        "Wampler5",
        StrdCorpusSuite::LinearRegression,
        "lls/Wampler5.dat",
        2_425,
        "c4e260a6638db61287bfea75a2dd3b324c86c5d40931ddc6d8fab0c0305ffa39",
    ),
    // Nonlinear least-squares regression (27)
    pinned(
        "Misra1a",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Misra1a.dat",
        1_932,
        "9d3194d537d9a6bbf92eb819504070923a1885d0191f40b6b3aaeedd130b98e2",
    ),
    pinned(
        "Chwirut2",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Chwirut2.dat",
        3_178,
        "c37e5522a34f9a57e18d7d10f8736202eabc0c33b8e7ad0010e618db04b57411",
    ),
    pinned(
        "Chwirut1",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Chwirut1.dat",
        7_835,
        "d9a055dfe5af71a8754c00f073ef00f8fed2e3fd1c6fd20cea8fd62d7cc3ed84",
    ),
    pinned(
        "Lanczos3",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Lanczos3.dat",
        2_661,
        "bc998624541c49fedee08346ee2c1d495741c4ea1dd15c052d7c743ab3cf0efe",
    ),
    pinned(
        "Gauss1",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Gauss1.dat",
        8_417,
        "20d0b85ffc73681ce9bdd69c566e53969a65e0e2192c3de16ad4a1dcfdca6c27",
    ),
    pinned(
        "Gauss2",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Gauss2.dat",
        8_416,
        "24fe86801c35c4f86140cced68894e31d73b6ed5738cf698474144f94bae3640",
    ),
    pinned(
        "DanWood",
        StrdCorpusSuite::NonlinearRegression,
        "nls/DanWood.dat",
        2_073,
        "5eae83de8b34e34c631a5738d640183b0628913e728437ae77f066e832186e24",
    ),
    pinned(
        "Misra1b",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Misra1b.dat",
        1_924,
        "dd27e310f787dcbc2addc50b9f8e5e3b82155f732926898b1858e4d08eead6a8",
    ),
    pinned(
        "Kirby2",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Kirby2.dat",
        6_073,
        "cbd288dd9d8586ec97ca408277d1a27ef41b19b543c158d594af189de80ae61c",
    ),
    pinned(
        "Hahn1",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Hahn1.dat",
        10_053,
        "7e885af48e18a222033a66cb22b491f0f5bc761b652142c0d9b28132daf63956",
    ),
    pinned(
        "Nelson",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Nelson.dat",
        7_194,
        "82fe77ef1a79a4441e7af5083680fc5046f9eb29b33bc2f2b76c478ecbe976d7",
    ),
    pinned("MGH17", StrdCorpusSuite::NonlinearRegression, "nls/MGH17.dat", 3_178, "75edb5522ee639794ae1f0d0f4a1e3638e963c6854888b853e1f45cea3c2f4d7"),
    pinned(
        "Lanczos1",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Lanczos1.dat",
        3_032,
        "dc7a419a4d7e7c0891e378dcde953a3e5dd273618b64b2cee8301e51577b428a",
    ),
    pinned(
        "Lanczos2",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Lanczos2.dat",
        2_689,
        "be1ac732e725ef75298f082bcdedab9e2ee9395ad77d8fad2ef37c95e0421572",
    ),
    pinned(
        "Gauss3",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Gauss3.dat",
        8_418,
        "24842a9e795db59c2c6cbc4c11047f90faedad014f8146faa6b5555259958d6d",
    ),
    pinned(
        "Misra1c",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Misra1c.dat",
        1_932,
        "012f75aa0ff112afabdff73880e324ab28e2dc267b678537611c403b959dc941",
    ),
    pinned(
        "Misra1d",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Misra1d.dat",
        1_919,
        "4dfa39a381a5ffea3e5d9306af1bda9d53bc57226a8e6f0c19af6b9702580481",
    ),
    pinned(
        "Roszman1",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Roszman1.dat",
        2_573,
        "fb526231ae6b02929e3de54e072e259b3d87424089abd4b1015ffb946b1e0ab0",
    ),
    pinned("ENSO", StrdCorpusSuite::NonlinearRegression, "nls/ENSO.dat", 6_998, "0c644f45b0e586cb1bb0613bad68f63c704efadacd086e768468d33d83f3e5d9"),
    pinned("MGH09", StrdCorpusSuite::NonlinearRegression, "nls/MGH09.dat", 2_393, "6419da83fd9ab34615769413dc2a6d35b11372f512a9401628a1565067c51d10"),
    pinned(
        "Thurber",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Thurber.dat",
        3_128,
        "806b8d00a5365eff907561db796fec3af8a26a5f353297d4311fd46ed5b0146f",
    ),
    pinned(
        "BoxBOD",
        StrdCorpusSuite::NonlinearRegression,
        "nls/BoxBOD.dat",
        1_784,
        "fc44fbed04e3be16df6498e722d4c8aedfd0af210110bc396d9f99acecd77e21",
    ),
    pinned("Rat42", StrdCorpusSuite::NonlinearRegression, "nls/Rat42.dat", 1_945, "fc97dd1f94fbcb381495065a053e279f23588a9ceb243276f2a2c36c8225985b"),
    pinned("MGH10", StrdCorpusSuite::NonlinearRegression, "nls/MGH10.dat", 2_421, "a0eb140580fdd6dcf6fba2cbbd81afa598c5226f1e6d702d1313c72a7a6e6fb4"),
    pinned(
        "Eckerle4",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Eckerle4.dat",
        2_871,
        "8ec2133b29a0a7cecbfccd51f7c61ee432f73c460bf5ce7106d672642e57a61e",
    ),
    pinned("Rat43", StrdCorpusSuite::NonlinearRegression, "nls/Rat43.dat", 2_168, "36f08908d0f09b1d075e3701ca567701535c58ac0d795ff68fb3c04e172c2c5d"),
    pinned(
        "Bennett5",
        StrdCorpusSuite::NonlinearRegression,
        "nls/Bennett5.dat",
        7_088,
        "b34d160605b959c301d13aac6627b693743d32ad98a6c4f830a3f728cdff0f52",
    ),
];

/// Errors returned when verifying or parsing pinned NIST StRD `.dat` files from disk.
#[derive(Debug)]
pub enum StrdCorpusLoadError {
    UnknownDataset { suite: StrdCorpusSuite, name: String },
    Verify { name: &'static str, source: CorpusError },
    Parse { name: &'static str, source: StrdParseError },
}

impl fmt::Display for StrdCorpusLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDataset { suite, name } => {
                write!(f, "unknown NIST StRD dataset `{name}` in suite `{}`", suite.subdir())
            }
            Self::Verify { name, source } => write!(f, "{name}: corpus verification failed: {source}"),
            Self::Parse { name, source } => write!(f, "{name}: corpus parse failed: {source}"),
        }
    }
}

impl std::error::Error for StrdCorpusLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnknownDataset { .. } => None,
            Self::Verify { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

/// Resolve the default NIST StRD corpus directory (`LUMENCRAFT_CORPUS_DIR` env override, or
/// `<workspace_root>/corpus/nist-strd`).
#[must_use]
pub fn default_nist_strd_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("LUMENCRAFT_CORPUS_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().and_then(Path::parent).map_or_else(|| PathBuf::from("corpus/nist-strd"), |root| root.join("corpus/nist-strd"))
}

/// Check whether all 58 pinned NIST StRD `.dat` files exist under `dir`.
#[must_use]
pub fn corpus_available(dir: &Path) -> bool {
    dir.is_dir() && PINNED_NIST_STRD_CORPUS.iter().all(|spec| dir.join(spec.manifest.relative_path).is_file())
}

/// Look up a pinned dataset entry by suite and case-insensitive dataset name.
#[must_use]
pub fn find_pinned_spec(suite: StrdCorpusSuite, name: &str) -> Option<&'static PinnedStrdSpec> {
    PINNED_NIST_STRD_CORPUS.iter().find(|spec| spec.suite == suite && spec.name.eq_ignore_ascii_case(name))
}

fn load_verified_spec_text(dir: &Path, spec: &PinnedStrdSpec) -> Result<String, StrdCorpusLoadError> {
    load_verified_corpus_file(dir, spec.manifest).map_err(|source| StrdCorpusLoadError::Verify { name: spec.name, source })
}

fn load_suite_all<T, F>(dir: &Path, suite: StrdCorpusSuite, parse: F) -> Result<Vec<T>, StrdCorpusLoadError>
where
    F: Fn(&str) -> Result<T, StrdParseError>,
{
    let mut out = Vec::new();
    for spec in PINNED_NIST_STRD_CORPUS.iter().filter(|entry| entry.suite == suite) {
        let text = load_verified_spec_text(dir, spec)?;
        let parsed = parse(&text).map_err(|source| StrdCorpusLoadError::Parse { name: spec.name, source })?;
        out.push(parsed);
    }
    Ok(out)
}

fn load_suite_by_name<T, F>(dir: &Path, suite: StrdCorpusSuite, name: &str, parse: F) -> Result<T, StrdCorpusLoadError>
where
    F: Fn(&str) -> Result<T, StrdParseError>,
{
    let Some(spec) = find_pinned_spec(suite, name) else {
        return Err(StrdCorpusLoadError::UnknownDataset { suite, name: name.to_owned() });
    };
    let text = load_verified_spec_text(dir, spec)?;
    parse(&text).map_err(|source| StrdCorpusLoadError::Parse { name: spec.name, source })
}

/// Load and SHA-256-verify a single NIST StRD Univariate `.dat` dataset by name.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] if the dataset name is unknown, SHA-256/byte-size fails, or parsing fails.
pub fn load_corpus_univariate(dir: &Path, name: &str) -> Result<UnivariateDataset, StrdCorpusLoadError> {
    load_suite_by_name(dir, StrdCorpusSuite::Univariate, name, parse_univariate_dat)
}

/// Load and SHA-256-verify a single NIST StRD One-Way ANOVA `.dat` dataset by name.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] if the dataset name is unknown, SHA-256/byte-size fails, or parsing fails.
pub fn load_corpus_anova(dir: &Path, name: &str) -> Result<AnovaDataset, StrdCorpusLoadError> {
    load_suite_by_name(dir, StrdCorpusSuite::Anova, name, parse_anova_dat)
}

/// Load and SHA-256-verify a single NIST StRD Linear Regression `.dat` dataset by name.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] if the dataset name is unknown, SHA-256/byte-size fails, or parsing fails.
pub fn load_corpus_linear(dir: &Path, name: &str) -> Result<LinearRegressionDataset, StrdCorpusLoadError> {
    load_suite_by_name(dir, StrdCorpusSuite::LinearRegression, name, parse_linear_regression_dat)
}

/// Load and SHA-256-verify a single NIST StRD Nonlinear Regression `.dat` dataset by name.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] if the dataset name is unknown, SHA-256/byte-size fails, or parsing fails.
pub fn load_corpus_nonlinear(dir: &Path, name: &str) -> Result<NonlinearDataset, StrdCorpusLoadError> {
    load_suite_by_name(dir, StrdCorpusSuite::NonlinearRegression, name, parse_nonlinear_dat)
}

/// Load and SHA-256-verify all 9 NIST StRD Univariate `.dat` datasets under `dir`.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] on any missing file, SHA-256/byte-size mismatch, or parse failure.
pub fn load_corpus_univariate_all(dir: &Path) -> Result<Vec<UnivariateDataset>, StrdCorpusLoadError> {
    load_suite_all(dir, StrdCorpusSuite::Univariate, parse_univariate_dat)
}

/// Load and SHA-256-verify all 11 NIST StRD One-Way ANOVA `.dat` datasets under `dir`.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] on any missing file, SHA-256/byte-size mismatch, or parse failure.
pub fn load_corpus_anova_all(dir: &Path) -> Result<Vec<AnovaDataset>, StrdCorpusLoadError> {
    load_suite_all(dir, StrdCorpusSuite::Anova, parse_anova_dat)
}

/// Load and SHA-256-verify all 11 NIST StRD Linear Regression `.dat` datasets under `dir`.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] on any missing file, SHA-256/byte-size mismatch, or parse failure.
pub fn load_corpus_linear_all(dir: &Path) -> Result<Vec<LinearRegressionDataset>, StrdCorpusLoadError> {
    load_suite_all(dir, StrdCorpusSuite::LinearRegression, parse_linear_regression_dat)
}

/// Load and SHA-256-verify all 27 NIST StRD Nonlinear Regression `.dat` datasets under `dir`.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] on any missing file, SHA-256/byte-size mismatch, or parse failure.
pub fn load_corpus_nonlinear_all(dir: &Path) -> Result<Vec<NonlinearDataset>, StrdCorpusLoadError> {
    load_suite_all(dir, StrdCorpusSuite::NonlinearRegression, parse_nonlinear_dat)
}

/// Verify SHA-256 digests, byte counts, and ASCII `.dat` parses across all 58 pinned NIST StRD files.
///
/// # Errors
/// Returns [`StrdCorpusLoadError`] if any file fails verification or parsing.
pub fn verify_pinned_corpus_all(dir: &Path) -> Result<usize, StrdCorpusLoadError> {
    let univ = load_corpus_univariate_all(dir)?;
    let anova = load_corpus_anova_all(dir)?;
    let lls = load_corpus_linear_all(dir)?;
    let nls = load_corpus_nonlinear_all(dir)?;
    Ok(univ.len() + anova.len() + lls.len() + nls.len())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        PINNED_NIST_STRD_CORPUS, StrdCorpusSuite, corpus_available, default_nist_strd_dir, load_corpus_anova_all, load_corpus_linear_all,
        load_corpus_nonlinear, load_corpus_nonlinear_all, load_corpus_univariate, load_corpus_univariate_all, verify_pinned_corpus_all,
    };

    fn require_corpus_env() -> bool {
        std::env::var_os("LUMENCRAFT_REQUIRE_CORPUS").is_some_and(|v| v == "1")
    }

    #[test]
    fn pinned_catalog_covers_58_unique_nist_strd_entries() {
        assert_eq!(PINNED_NIST_STRD_CORPUS.len(), 58);
        let univ = PINNED_NIST_STRD_CORPUS.iter().filter(|s| s.suite == StrdCorpusSuite::Univariate).count();
        let anova = PINNED_NIST_STRD_CORPUS.iter().filter(|s| s.suite == StrdCorpusSuite::Anova).count();
        let lls = PINNED_NIST_STRD_CORPUS.iter().filter(|s| s.suite == StrdCorpusSuite::LinearRegression).count();
        let nls = PINNED_NIST_STRD_CORPUS.iter().filter(|s| s.suite == StrdCorpusSuite::NonlinearRegression).count();
        assert_eq!((univ, anova, lls, nls), (9, 11, 11, 27));

        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for spec in PINNED_NIST_STRD_CORPUS {
            assert!(names.insert(spec.name), "duplicate pinned dataset name: {}", spec.name);
            assert!(paths.insert(spec.manifest.relative_path), "duplicate pinned path: {}", spec.manifest.relative_path);
            assert_eq!(spec.manifest.sha256_hex.len(), 64);
            assert!(spec.manifest.sha256_hex.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(spec.manifest.expected_bytes > 0);
        }
    }

    fn verify_sha256sums_manifest_matches_pinned(dir: &std::path::Path) {
        let manifest_text = std::fs::read_to_string(dir.join("sha256sums.txt")).unwrap();
        let manifest_rows: Vec<(&str, usize, &str)> = manifest_text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                assert_eq!(parts.len(), 3, "unexpected sha256sums.txt line: {line}");
                (parts[0], parts[1].parse::<usize>().unwrap(), parts[2])
            })
            .collect();
        assert_eq!(manifest_rows.len(), 58);
        for ((sha, bytes, rel), spec) in manifest_rows.iter().zip(PINNED_NIST_STRD_CORPUS.iter()) {
            assert_eq!(*sha, spec.manifest.sha256_hex);
            assert_eq!(*bytes, spec.manifest.expected_bytes);
            assert_eq!(*rel, spec.manifest.relative_path);
        }
    }

    fn assert_nonlinear_dataset_invariants(ds: &crate::nist_strd::NonlinearDataset) {
        assert!(!ds.param_names.is_empty(), "{} param_names empty", ds.name);
        assert_eq!(ds.start_1.len(), ds.param_names.len(), "{} start_1 len", ds.name);
        assert_eq!(ds.start_2.len(), ds.param_names.len(), "{} start_2 len", ds.name);
        assert_eq!(ds.certified_betas.len(), ds.param_names.len(), "{} betas len", ds.name);
        assert_eq!(ds.certified_se_betas.len(), ds.param_names.len(), "{} se_betas len", ds.name);
        assert_eq!(ds.y.len(), ds.predictors.len(), "{} y vs predictors len", ds.name);
        assert!(!ds.y.is_empty(), "{} y empty", ds.name);

        // Upstream NIST `Rat43.dat` prints `Degrees of Freedom: 9` alongside 15 observations & 4 parameters
        // (its published Residual Standard Deviation equals sqrt(RSS / 11)).
        if ds.name == "Rat43" {
            assert_eq!(ds.y.len(), 15);
            assert_eq!(ds.param_names.len(), 4);
            assert_eq!(ds.certified_df, 9);
        } else {
            assert_eq!(ds.certified_df + ds.param_names.len(), ds.y.len(), "{} df + p == n", ds.name);
        }

        assert!(ds.certified_rss.is_finite(), "{} rss finite", ds.name);
        assert!(ds.certified_residual_sd.is_finite(), "{} rsd finite", ds.name);
        let scalars =
            ds.start_1.iter().chain(ds.start_2.iter()).chain(ds.certified_betas.iter()).chain(ds.certified_se_betas.iter()).chain(ds.y.iter());
        assert!(scalars.copied().all(f64::is_finite), "{} non-finite scalar", ds.name);
        assert!(ds.predictors.iter().all(|row| !row.is_empty() && row.iter().copied().all(f64::is_finite)), "{} invalid predictors", ds.name);
    }

    #[test]
    fn on_disk_58_file_corpus_verifies_sha256_and_parses_all_suites() {
        let dir = default_nist_strd_dir();
        if !corpus_available(&dir) {
            assert!(!require_corpus_env(), "LUMENCRAFT_REQUIRE_CORPUS=1 set, but NIST StRD corpus is missing or incomplete at {}", dir.display());
            return;
        }

        verify_sha256sums_manifest_matches_pinned(&dir);
        assert_eq!(verify_pinned_corpus_all(&dir).unwrap(), 58);

        let univ = load_corpus_univariate_all(&dir).unwrap();
        assert_eq!(univ.len(), 9);
        assert_eq!(load_corpus_univariate(&dir, "numacc4").unwrap().values.len(), 1001);

        let anova = load_corpus_anova_all(&dir).unwrap();
        assert_eq!(anova.len(), 11);

        let lls = load_corpus_linear_all(&dir).unwrap();
        assert_eq!(lls.len(), 11);

        let nls = load_corpus_nonlinear_all(&dir).unwrap();
        assert_eq!(nls.len(), 27);
        assert_eq!(load_corpus_nonlinear(&dir, "Bennett5").unwrap().y.len(), 154);

        for ds in &nls {
            assert_nonlinear_dataset_invariants(ds);
        }
    }
}
