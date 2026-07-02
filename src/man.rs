use clap::CommandFactory;

use crate::cli::Cli;

/// Render the bastion man page to a byte buffer (pure — no filesystem access).
/// The output is valid roff that `man -l` can render directly.
pub fn render_man() -> std::io::Result<Vec<u8>> {
    let cmd = Cli::command();
    let mut buf = Vec::new();
    clap_mangen::Man::new(cmd).render(&mut buf)?;
    Ok(buf)
}

/// Write `bastion.1` (plus one page per subcommand) into `out_dir`.
/// Thin I/O shell over `render_man` — only this function touches the filesystem.
pub fn write_man_pages(out_dir: &std::path::Path) -> std::io::Result<()> {
    let cmd = Cli::command();
    clap_mangen::generate_to(cmd, out_dir)
}

/// Dispatch `bastion man [--out <dir>]`.
pub fn run(out: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    match out {
        None => {
            let bytes = render_man()?;
            let text = String::from_utf8_lossy(&bytes);
            print!("{text}");
            Ok(())
        }
        Some(dir) => {
            std::fs::create_dir_all(&dir)?;
            write_man_pages(&dir)?;
            println!("man pages written to {}", dir.display());
            Ok(())
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_man_is_non_empty() {
        let bytes = render_man().expect("render_man should succeed");
        assert!(!bytes.is_empty(), "man output must not be empty");
    }

    #[test]
    fn render_man_contains_th_header() {
        let bytes = render_man().expect("render_man should succeed");
        let text = String::from_utf8(bytes).expect("man output should be valid UTF-8");
        assert!(
            text.contains(".TH"),
            "man output should contain a .TH header; got:\n{text}"
        );
    }

    #[test]
    fn render_man_contains_command_name() {
        let bytes = render_man().expect("render_man should succeed");
        let text = String::from_utf8(bytes).expect("man output should be valid UTF-8");
        // Command name appears in the .TH line
        assert!(
            text.to_ascii_uppercase().contains("BASTION"),
            "man output should reference BASTION; got:\n{text}"
        );
    }

    #[test]
    fn render_man_is_deterministic() {
        let a = render_man().expect("first render_man should succeed");
        let b = render_man().expect("second render_man should succeed");
        assert_eq!(a, b, "render_man must be deterministic across two calls");
    }
}
