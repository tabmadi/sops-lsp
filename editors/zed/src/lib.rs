use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, LanguageServerId, Result, serde_json};

const SERVER_BINARY: &str = "sops-lsp";
/// The id `extension.toml` registers, which is also the key under `lsp` in Zed's settings.
const SERVER_NAME: &str = "sops-lsp";
const REPOSITORY: &str = "tabmadi/sops-lsp";

struct SopsExtension {
    cached_binary_path: Option<String>,
}

impl SopsExtension {
    /// A binary on PATH wins over a downloaded one, so a contributor's working build is what
    /// runs without uninstalling anything.
    fn server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        if let Some(path) = worktree.which(SERVER_BINARY) {
            return Ok(path);
        }
        if let Some(path) = &self.cached_binary_path
            && std::fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = zed::latest_github_release(
            REPOSITORY,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;
        let asset_name = asset_name()?;
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("{} publishes no {asset_name}", release.version))?;

        let version_dir = format!("{SERVER_BINARY}-{}", release.version);
        let binary_path = format!("{version_dir}/{SERVER_BINARY}");
        if !std::fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
            zed::download_file(
                &asset.download_url,
                &version_dir,
                zed::DownloadedFileType::GzipTar,
            )?;
            zed::make_file_executable(&binary_path)?;
            // Every other version, so a download that fails leaves the working one in place.
            for entry in std::fs::read_dir(".").map_err(|err| format!("list work dir: {err}"))? {
                let entry = entry.map_err(|err| format!("read work dir entry: {err}"))?;
                if entry.file_name().to_str() != Some(&version_dir) {
                    std::fs::remove_dir_all(entry.path()).ok();
                }
            }
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

fn asset_name() -> Result<String> {
    let (os, arch) = zed::current_platform();
    let arch = match arch {
        zed::Architecture::Aarch64 => "aarch64",
        zed::Architecture::X8664 => "x86_64",
        zed::Architecture::X86 => return Err("32-bit x86 is not a release target".into()),
    };
    let os = match os {
        zed::Os::Mac => "apple-darwin",
        zed::Os::Linux => "unknown-linux-gnu",
        zed::Os::Windows => "pc-windows-msvc",
    };
    Ok(format!("{SERVER_BINARY}-{arch}-{os}.tar.gz"))
}

impl zed::Extension for SopsExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        // `lsp.sops-lsp.binary` overrides the server itself, the way every other Zed language
        // server is overridden.
        let configured = LspSettings::for_worktree(SERVER_NAME, worktree)
            .ok()
            .and_then(|settings| settings.binary);
        Ok(zed::Command {
            command: match configured.as_ref().and_then(|binary| binary.path.clone()) {
                Some(path) => path,
                None => self.server_binary_path(language_server_id, worktree)?,
            },
            args: configured
                .and_then(|binary| binary.arguments)
                .unwrap_or_default(),
            // The server shells out to `sops`, which reads SOPS_AGE_KEY_FILE, GPG_TTY and the
            // rest of the caller's key configuration from here.
            env: worktree.shell_env(),
        })
    }

    /// `sops` is often absent from the PATH a GUI editor inherits, and a version-managed install
    /// is reachable only through a shim. What the worktree can resolve is the default; the
    /// setting is what overrides it.
    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<serde_json::Value>> {
        let configured = LspSettings::for_worktree(SERVER_NAME, worktree)
            .ok()
            .and_then(|settings| settings.initialization_options);
        if let Some(options) = configured
            && options.pointer("/sops/path").is_some()
        {
            return Ok(Some(options));
        }
        Ok(worktree
            .which("sops")
            .map(|path| serde_json::json!({ "sops": { "path": path } })))
    }
}

zed::register_extension!(SopsExtension);
