//! Shared command-line argument helpers for `cmvs-cli` binaries.

use std::ffi::OsStr;

/// Returns `true` when `-h` or `--help` appears in `args` before any `--` separator.
pub fn wants_help<I, T>(args: I) -> bool
where
    I: IntoIterator<Item = T>,
    T: AsRef<OsStr>,
{
    for arg in args {
        let arg = arg.as_ref();
        if arg == OsStr::new("--") {
            break;
        }
        if arg == OsStr::new("-h") || arg == OsStr::new("--help") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::wants_help;

    #[test]
    fn detects_short_and_long_help_only_before_double_dash() {
        assert!(!wants_help(Vec::<&str>::new()));
        assert!(wants_help(["-h"]));
        assert!(wants_help(["--help"]));
        assert!(wants_help(["input.ps3", "--payload", "-h"]));
        assert!(wants_help(["--unknown", "--help", "--", "file"]));
        assert!(!wants_help(["--", "-h"]));
        assert!(!wants_help(["--", "--help"]));
        assert!(!wants_help(["--payload", "out.bin", "--", "--help"]));
    }
}
