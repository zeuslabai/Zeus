//! macOS .pkg installer builder
//!
//! Builds a product archive with 8 component packages:
//!   1. Zeus CLI          - /usr/local/bin/zeus (via postinstall)
//!   2. Zeus Setup        - /usr/local/bin/zeus-setup (via postinstall)
//!   3. Desktop App       - /Applications/Zeus.app
//!   4. Gateway Service   - launchd plist
//!   5. Workspace Setup   - ~/.zeus/ directory
//!   6. MCP Config        - Claude MCP integration
//!   7. Web Frontend      - /usr/local/share/zeus/web/
//!   8. Shell Completions - bash/zsh/fish completions

use crate::event::ProgressEvent;
use crate::ops::util::{run_command, run_command_capture};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Options for the package command
pub struct PackageOpts {
    pub cli_only: bool,
    pub app_only: bool,
    pub version: String,
    pub sign_identity: Option<String>,
    pub notarize: bool,
    pub apple_id: Option<String>,
    pub team_id: Option<String>,
    pub dist_dir: PathBuf,
    pub project_root: PathBuf,
    pub skip_build: bool,
}

/// Component package metadata
struct Component {
    name: &'static str,
    identifier: &'static str,
    version: String,
    install_location: &'static str,
    payload_dir: Option<PathBuf>,
    scripts_dir: Option<PathBuf>,
}

pub async fn run(opts: PackageOpts, tx: mpsc::Sender<ProgressEvent>) -> Result<()> {
    let start = std::time::Instant::now();
    let staging = opts.dist_dir.join("staging");
    let pkgs_dir = opts.dist_dir.join("packages");
    let resources_dir = opts.project_root.join("packaging/macos/resources");
    let scripts_base = opts.project_root.join("packaging/macos/scripts");

    // Determine which components to build
    let include_desktop = !opts.cli_only;
    let include_extras = !opts.cli_only && !opts.app_only;
    let include_core = !opts.app_only;

    // Calculate total steps
    let mut total = 2; // validate + stage
    if !opts.skip_build && include_core {
        total += 1; // build binaries
    }
    if include_desktop && !opts.skip_build {
        total += 1; // build desktop
    }
    if include_extras && !opts.skip_build {
        total += 2; // build web + generate completions
    }
    total += 3; // build component pkgs + distribution xml + product pkg
    if opts.sign_identity.is_some() {
        total += 1; // sign (handled by productbuild)
    }
    if opts.notarize {
        total += 1; // notarize
    }
    total += 1; // verify

    let mut step = 0;

    // ── Step 1: Validate ──────────────────────────────────────────────
    send_step(&tx, "Validate environment", step, total).await?;
    validate_environment(&opts).await?;
    send_done(&tx, "Validate environment", "OK").await?;
    step += 1;

    // ── Step 2: Build binaries ────────────────────────────────────────
    if !opts.skip_build && include_core {
        send_step(&tx, "Build release binaries", step, total).await?;
        run_command(
            &tx,
            "cargo",
            &["build", "--release", "--bin", "zeus", "--bin", "zeus-setup"],
            &opts.project_root,
        )
        .await?;
        send_done(&tx, "Build release binaries", "OK").await?;
        step += 1;
    }

    // ── Step 3: Build Desktop app ─────────────────────────────────────
    if include_desktop && !opts.skip_build {
        send_step(&tx, "Build Desktop app", step, total).await?;
        tx.send(ProgressEvent::LogLine(
            "Skipping xcodebuild (use pre-built Zeus.app or --skip-build)".into(),
        ))
        .await?;
        send_done(
            &tx,
            "Build Desktop app",
            "Skipped (use --skip-build with pre-built app)",
        )
        .await?;
        step += 1;
    }

    // ── Step 4: Build web frontend ────────────────────────────────────
    if include_extras && !opts.skip_build {
        send_step(&tx, "Build web frontend", step, total).await?;
        let web_dir = opts.project_root.join("apps/ZeusWeb");
        if web_dir.exists() {
            run_command(&tx, "trunk", &["build", "--release"], &web_dir).await?;
            send_done(&tx, "Build web frontend", "OK").await?;
        } else {
            send_done(
                &tx,
                "Build web frontend",
                "Skipped (apps/ZeusWeb not found)",
            )
            .await?;
        }
        step += 1;

        // ── Step 5: Generate completions ──────────────────────────────
        send_step(&tx, "Generate shell completions", step, total).await?;
        let zeus_bin = opts.project_root.join("target/release/zeus");
        if zeus_bin.exists() {
            let comp_dir = staging.join("completions");
            std::fs::create_dir_all(comp_dir.join("zsh/site-functions"))?;
            std::fs::create_dir_all(comp_dir.join("bash-completion/completions"))?;
            std::fs::create_dir_all(comp_dir.join("fish/vendor_completions.d"))?;

            for (shell, path) in [
                ("zsh", "zsh/site-functions/_zeus"),
                ("bash", "bash-completion/completions/zeus"),
                ("fish", "fish/vendor_completions.d/zeus.fish"),
            ] {
                let output = run_command_capture(
                    zeus_bin.to_str().unwrap(),
                    &["completion", shell],
                    &opts.project_root,
                )
                .await;
                match output {
                    Ok(content) => {
                        std::fs::write(comp_dir.join(path), content)?;
                    }
                    Err(e) => {
                        tx.send(ProgressEvent::LogLine(format!(
                            "Warning: {} completion generation failed: {}",
                            shell, e
                        )))
                        .await?;
                    }
                }
            }
            send_done(&tx, "Generate shell completions", "OK").await?;
        } else {
            send_done(
                &tx,
                "Generate shell completions",
                "Skipped (zeus binary not found)",
            )
            .await?;
        }
        step += 1;
    }

    // ── Step 6: Stage files ───────────────────────────────────────────
    send_step(&tx, "Stage files", step, total).await?;
    stage_files(
        &opts,
        &staging,
        include_core,
        include_desktop,
        include_extras,
        &tx,
    )
    .await?;
    send_done(&tx, "Stage files", "OK").await?;
    step += 1;

    // ── Step 7: Build component .pkgs ─────────────────────────────────
    send_step(&tx, "Build component packages", step, total).await?;
    std::fs::create_dir_all(&pkgs_dir)?;
    let components = build_component_packages(
        &opts,
        &staging,
        &pkgs_dir,
        &scripts_base,
        include_core,
        include_desktop,
        include_extras,
        &tx,
    )
    .await?;
    send_done(
        &tx,
        "Build component packages",
        &format!("{} components built", components.len()),
    )
    .await?;
    step += 1;

    // ── Step 8: Generate Distribution.xml ─────────────────────────────
    send_step(&tx, "Generate Distribution.xml", step, total).await?;
    let dist_xml = generate_distribution_xml(&opts.version, &components, include_desktop);
    let dist_xml_path = opts.dist_dir.join("Distribution.xml");
    std::fs::write(&dist_xml_path, &dist_xml)?;
    tx.send(ProgressEvent::LogLine(format!(
        "Wrote {}",
        dist_xml_path.display()
    )))
    .await?;
    send_done(&tx, "Generate Distribution.xml", "OK").await?;
    step += 1;

    // ── Step 9: Build product .pkg ────────────────────────────────────
    send_step(&tx, "Build product package", step, total).await?;
    let pkg_name = if opts.cli_only {
        format!("Zeus-CLI-{}.pkg", opts.version)
    } else {
        format!("Zeus-{}.pkg", opts.version)
    };
    let output_pkg = opts.dist_dir.join(&pkg_name);

    let mut product_args = vec![
        "--distribution",
        dist_xml_path.to_str().unwrap(),
        "--package-path",
        pkgs_dir.to_str().unwrap(),
    ];

    // Resources directory for welcome/license/conclusion screens
    let res_str;
    if resources_dir.exists() {
        res_str = resources_dir.to_string_lossy().to_string();
        product_args.push("--resources");
        product_args.push(&res_str);
    }

    let sign_str;
    if let Some(ref identity) = opts.sign_identity {
        sign_str = identity.clone();
        product_args.push("--sign");
        product_args.push(&sign_str);
    }

    let output_str = output_pkg.to_string_lossy().to_string();
    product_args.push(&output_str);

    run_command(&tx, "productbuild", &product_args, &opts.project_root).await?;
    send_done(
        &tx,
        "Build product package",
        &format!("{}", output_pkg.display()),
    )
    .await?;
    step += 1;

    // ── Step 10: Notarize ─────────────────────────────────────────────
    if opts.notarize {
        send_step(&tx, "Notarize package", step, total).await?;
        notarize_package(&output_pkg, &opts, &tx).await?;
        send_done(&tx, "Notarize package", "Notarized + stapled").await?;
        step += 1;
    }

    // ── Step 11: Verify ───────────────────────────────────────────────
    send_step(&tx, "Verify package", step, total).await?;
    if opts.sign_identity.is_some() {
        run_command(
            &tx,
            "pkgutil",
            &["--check-signature", output_pkg.to_str().unwrap()],
            &opts.project_root,
        )
        .await?;
        send_done(&tx, "Verify package", "Signature valid").await?;
    } else {
        // List payload to verify structure
        run_command(
            &tx,
            "pkgutil",
            &["--payload-files", output_pkg.to_str().unwrap()],
            &opts.project_root,
        )
        .await
        .ok(); // payload-files may fail on product pkgs, that's OK
        send_done(
            &tx,
            "Verify package",
            "Unsigned (use --sign for distribution)",
        )
        .await?;
    }

    tx.send(ProgressEvent::Finished {
        success: true,
        elapsed: start.elapsed(),
        summary: format!("Package created: {}", output_pkg.display()),
    })
    .await?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────

async fn send_step(
    tx: &mpsc::Sender<ProgressEvent>,
    name: &str,
    index: usize,
    total: usize,
) -> Result<()> {
    tx.send(ProgressEvent::StepStarted {
        name: name.to_string(),
        index,
        total,
    })
    .await?;
    Ok(())
}

async fn send_done(tx: &mpsc::Sender<ProgressEvent>, name: &str, message: &str) -> Result<()> {
    tx.send(ProgressEvent::StepCompleted {
        name: name.to_string(),
        message: message.to_string(),
    })
    .await?;
    Ok(())
}

/// Validate that required tools exist and options are consistent
async fn validate_environment(opts: &PackageOpts) -> Result<()> {
    // Check pkgbuild exists
    which_exists("pkgbuild").context("pkgbuild not found — Xcode Command Line Tools required")?;
    which_exists("productbuild")
        .context("productbuild not found — Xcode Command Line Tools required")?;

    // If notarize, check required fields
    if opts.notarize {
        if opts.sign_identity.is_none() {
            bail!("--notarize requires --sign");
        }
        if opts.apple_id.is_none() {
            bail!("--notarize requires --apple-id (or ZEUS_APPLE_ID env)");
        }
        if opts.team_id.is_none() {
            bail!("--notarize requires --team-id (or ZEUS_TEAM_ID env)");
        }
        which_exists("xcrun").context("xcrun not found — Xcode required for notarization")?;
    }

    // If signing, verify identity exists in keychain
    if let Some(ref identity) = opts.sign_identity {
        let output = std::process::Command::new("security")
            .args(["find-identity", "-v", "-p", "basic"])
            .output()
            .context("Failed to query keychain")?;
        let identities = String::from_utf8_lossy(&output.stdout);
        if !identities.contains(identity) {
            bail!(
                "Signing identity '{}' not found in keychain. Available:\n{}",
                identity,
                identities
            );
        }
    }

    Ok(())
}

fn which_exists(cmd: &str) -> Result<()> {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| ())
        .context(format!("{} not found in PATH", cmd))
}

/// Stage all build artifacts into the staging directory
async fn stage_files(
    opts: &PackageOpts,
    staging: &Path,
    include_core: bool,
    include_desktop: bool,
    include_extras: bool,
    tx: &mpsc::Sender<ProgressEvent>,
) -> Result<()> {
    // Clean previous staging
    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }

    // CLI binary — staged to /usr/local/share/zeus/bin/, postinstall copies to /usr/local/bin/
    if include_core {
        let cli_dir = staging.join("cli/usr/local/share/zeus/bin");
        std::fs::create_dir_all(&cli_dir)?;
        let zeus_bin = opts.project_root.join("target/release/zeus");
        if zeus_bin.exists() {
            std::fs::copy(&zeus_bin, cli_dir.join("zeus"))?;
            tx.send(ProgressEvent::LogLine("Staged zeus CLI binary".into()))
                .await?;
        } else {
            bail!("zeus binary not found at target/release/zeus — build first or use --skip-build");
        }

        // Setup binary
        let setup_bin = opts.project_root.join("target/release/zeus-setup");
        if setup_bin.exists() {
            std::fs::copy(&setup_bin, cli_dir.join("zeus-setup"))?;
            tx.send(ProgressEvent::LogLine("Staged zeus-setup binary".into()))
                .await?;
        } else {
            tx.send(ProgressEvent::StepWarning {
                name: "Stage files".into(),
                message: "zeus-setup binary not found, skipping".into(),
            })
            .await?;
        }
    }

    // Desktop app
    if include_desktop {
        let desktop_dir = staging.join("desktop/Applications");
        std::fs::create_dir_all(&desktop_dir)?;
        // Look for pre-built app in common locations
        let app_candidates = [
            opts.project_root.join("build/Zeus.app"),
            opts.project_root
                .join("apps/ZeusDesktop/build/Release/Zeus.app"),
            PathBuf::from("/Applications/Zeus.app"),
        ];
        let mut found = false;
        for candidate in &app_candidates {
            if candidate.exists() {
                copy_dir_recursive(candidate, &desktop_dir.join("Zeus.app"))?;
                tx.send(ProgressEvent::LogLine(format!(
                    "Staged Zeus.app from {}",
                    candidate.display()
                )))
                .await?;
                found = true;
                break;
            }
        }
        if !found {
            tx.send(ProgressEvent::LogLine(
                "Warning: Zeus.app not found — desktop component will be empty".into(),
            ))
            .await?;
        }
    }

    // Gateway launchd plist
    if include_extras {
        let gw_dir = staging.join("gateway/usr/local/share/zeus/launchd");
        std::fs::create_dir_all(&gw_dir)?;
        std::fs::write(
            gw_dir.join("ai.zeus.gateway.plist"),
            generate_launchd_plist(),
        )?;
        tx.send(ProgressEvent::LogLine(
            "Staged gateway launchd plist".into(),
        ))
        .await?;
    }

    // Web frontend
    if include_extras {
        let web_src = opts.project_root.join("apps/ZeusWeb/dist");
        if web_src.exists() {
            let web_dir = staging.join("web/usr/local/share/zeus/web");
            std::fs::create_dir_all(&web_dir)?;
            copy_dir_recursive(&web_src, &web_dir)?;
            tx.send(ProgressEvent::LogLine("Staged web frontend".into()))
                .await?;
        } else {
            tx.send(ProgressEvent::LogLine(
                "Warning: Web frontend dist/ not found — component will be empty".into(),
            ))
            .await?;
        }
    }

    // Completions (already staged in the completions step)
    if include_extras {
        let comp_staging = staging.join("completions");
        if !comp_staging.exists() {
            std::fs::create_dir_all(comp_staging.join("usr/local/share/zsh/site-functions"))?;
            std::fs::create_dir_all(
                comp_staging.join("usr/local/share/bash-completion/completions"),
            )?;
            std::fs::create_dir_all(
                comp_staging.join("usr/local/share/fish/vendor_completions.d"),
            )?;
        }
        // Move completion files into proper payload structure
        let comp_src = staging.join("completions");
        let comp_payload = staging.join("completions-payload/usr/local/share");
        std::fs::create_dir_all(&comp_payload)?;

        for dir in ["zsh", "bash-completion", "fish"] {
            let src = comp_src.join(dir);
            if src.exists() {
                let dst = comp_payload.join(dir);
                copy_dir_recursive(&src, &dst)?;
            }
        }
    }

    Ok(())
}

/// Build individual component .pkg files using pkgbuild
#[allow(clippy::too_many_arguments)]
async fn build_component_packages(
    opts: &PackageOpts,
    staging: &Path,
    pkgs_dir: &Path,
    scripts_base: &Path,
    include_core: bool,
    include_desktop: bool,
    include_extras: bool,
    tx: &mpsc::Sender<ProgressEvent>,
) -> Result<Vec<Component>> {
    let mut components = Vec::new();

    // 1. Zeus CLI (payload: /usr/local/share/zeus/bin/, postinstall copies to /usr/local/bin/)
    if include_core {
        let cli_root = staging.join("cli");
        if cli_root.join("usr/local/share/zeus/bin/zeus").exists() {
            let scripts = scripts_base.join("cli");
            let comp = Component {
                name: "Zeus CLI",
                identifier: "com.zeus.cli",
                version: opts.version.clone(),
                install_location: "/",
                payload_dir: Some(cli_root.clone()),
                scripts_dir: if scripts.exists() {
                    Some(scripts)
                } else {
                    None
                },
            };
            build_one_pkg(&comp, pkgs_dir, tx).await?;
            components.push(comp);
        }
    }

    // 2. Zeus Setup (included in CLI payload, separate pkg-ref for Distribution.xml)
    if include_core {
        let cli_root = staging.join("cli");
        if cli_root
            .join("usr/local/share/zeus/bin/zeus-setup")
            .exists()
        {
            let comp = Component {
                name: "Zeus Setup",
                identifier: "com.zeus.setup",
                version: opts.version.clone(),
                install_location: "/",
                payload_dir: None, // already in CLI payload
                scripts_dir: None,
            };
            // No separate pkg needed — setup binary is in the CLI package
            // Just register it in components for Distribution.xml
            components.push(comp);
        }
    }

    // 3. Desktop App
    if include_desktop {
        let desktop_root = staging.join("desktop");
        if desktop_root.join("Applications/Zeus.app").exists() {
            let scripts = scripts_base.join("desktop");
            let comp = Component {
                name: "Zeus Desktop",
                identifier: "com.zeus.desktop",
                version: opts.version.clone(),
                install_location: "/",
                payload_dir: Some(desktop_root),
                scripts_dir: if scripts.exists() {
                    Some(scripts)
                } else {
                    None
                },
            };
            build_one_pkg(&comp, pkgs_dir, tx).await?;
            components.push(comp);
        }
    }

    // 4. Gateway Service
    if include_extras {
        let gw_root = staging.join("gateway");
        let scripts = scripts_base.join("gateway");
        let comp = Component {
            name: "Gateway Service",
            identifier: "com.zeus.gateway",
            version: opts.version.clone(),
            install_location: "/",
            payload_dir: Some(gw_root),
            scripts_dir: if scripts.exists() {
                Some(scripts)
            } else {
                None
            },
        };
        build_one_pkg(&comp, pkgs_dir, tx).await?;
        components.push(comp);
    }

    // 5. Workspace Setup (scripts-only)
    if include_extras {
        let scripts = scripts_base.join("workspace");
        if scripts.exists() {
            let comp = Component {
                name: "Workspace Setup",
                identifier: "com.zeus.workspace",
                version: opts.version.clone(),
                install_location: "/tmp",
                payload_dir: None,
                scripts_dir: Some(scripts),
            };
            build_one_pkg(&comp, pkgs_dir, tx).await?;
            components.push(comp);
        }
    }

    // 6. MCP Config (scripts-only)
    if include_extras {
        let scripts = scripts_base.join("mcp");
        if scripts.exists() {
            let comp = Component {
                name: "MCP Config",
                identifier: "com.zeus.mcp",
                version: opts.version.clone(),
                install_location: "/tmp",
                payload_dir: None,
                scripts_dir: Some(scripts),
            };
            build_one_pkg(&comp, pkgs_dir, tx).await?;
            components.push(comp);
        }
    }

    // 7. Web Frontend
    if include_extras {
        let web_root = staging.join("web");
        if web_root.exists() && web_root.join("usr/local/share/zeus/web").exists() {
            let comp = Component {
                name: "Web Frontend",
                identifier: "com.zeus.web",
                version: opts.version.clone(),
                install_location: "/",
                payload_dir: Some(web_root),
                scripts_dir: None,
            };
            build_one_pkg(&comp, pkgs_dir, tx).await?;
            components.push(comp);
        }
    }

    // 8. Shell Completions
    if include_extras {
        let comp_root = staging.join("completions-payload");
        if comp_root.exists() {
            let comp = Component {
                name: "Shell Completions",
                identifier: "com.zeus.completions",
                version: opts.version.clone(),
                install_location: "/",
                payload_dir: Some(comp_root),
                scripts_dir: None,
            };
            build_one_pkg(&comp, pkgs_dir, tx).await?;
            components.push(comp);
        }
    }

    Ok(components)
}

/// Build a single component .pkg
async fn build_one_pkg(
    comp: &Component,
    pkgs_dir: &Path,
    tx: &mpsc::Sender<ProgressEvent>,
) -> Result<()> {
    let pkg_file = pkgs_dir.join(format!("{}.pkg", comp.identifier));
    let mut args: Vec<String> = vec![
        "--identifier".to_string(),
        comp.identifier.to_string(),
        "--version".to_string(),
        comp.version.clone(),
        "--install-location".to_string(),
        comp.install_location.to_string(),
    ];

    if let Some(ref payload) = comp.payload_dir {
        args.push("--root".to_string());
        args.push(payload.to_string_lossy().to_string());
    } else {
        args.push("--nopayload".to_string());
    }

    if let Some(ref scripts) = comp.scripts_dir {
        args.push("--scripts".to_string());
        args.push(scripts.to_string_lossy().to_string());
    }

    args.push(pkg_file.to_string_lossy().to_string());

    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_command(tx, "pkgbuild", &args_refs, Path::new("/tmp")).await?;

    tx.send(ProgressEvent::LogLine(format!(
        "Built component: {} ({})",
        comp.name, comp.identifier
    )))
    .await?;

    Ok(())
}

/// Generate the Distribution.xml for productbuild
fn generate_distribution_xml(
    version: &str,
    components: &[Component],
    include_desktop: bool,
) -> String {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"no\"?>\n");
    xml.push_str("<installer-gui-script minSpecVersion=\"2\">\n");
    xml.push_str(&format!(
        "    <title>Zeus AI Assistant v{}</title>\n",
        version
    ));
    xml.push_str("    <organization>ai.zeus</organization>\n");
    xml.push_str(&format!(
        "    <product id=\"ai.zeus.pkg\" version=\"{}\" />\n",
        version
    ));
    xml.push('\n');

    // OS requirement
    xml.push_str("    <os-version min=\"14.0\" />\n");
    xml.push_str("    <allowed-os-versions>\n");
    xml.push_str("        <os-version min=\"14.0\" />\n");
    xml.push_str("    </allowed-os-versions>\n");
    xml.push('\n');

    // Resources
    xml.push_str("    <welcome file=\"welcome.html\" mime-type=\"text/html\" />\n");
    xml.push_str("    <license file=\"license.html\" mime-type=\"text/html\" />\n");
    xml.push_str("    <conclusion file=\"conclusion.html\" mime-type=\"text/html\" />\n");
    xml.push('\n');

    // Choices outline (component tree)
    xml.push_str("    <choices-outline>\n");
    xml.push_str("        <line choice=\"core\">\n");
    xml.push_str("            <line choice=\"com.zeus.cli\" />\n");
    xml.push_str("            <line choice=\"com.zeus.setup\" />\n");
    xml.push_str("        </line>\n");
    if include_desktop {
        xml.push_str("        <line choice=\"desktop\">\n");
        xml.push_str("            <line choice=\"com.zeus.desktop\" />\n");
        xml.push_str("        </line>\n");
    }
    xml.push_str("        <line choice=\"services\">\n");
    xml.push_str("            <line choice=\"com.zeus.gateway\" />\n");
    xml.push_str("            <line choice=\"com.zeus.workspace\" />\n");
    xml.push_str("            <line choice=\"com.zeus.mcp\" />\n");
    xml.push_str("        </line>\n");
    xml.push_str("        <line choice=\"extras\">\n");
    xml.push_str("            <line choice=\"com.zeus.web\" />\n");
    xml.push_str("            <line choice=\"com.zeus.completions\" />\n");
    xml.push_str("        </line>\n");
    xml.push_str("    </choices-outline>\n");
    xml.push('\n');

    // Group choices
    xml.push_str("    <choice id=\"core\" title=\"Zeus Core\" description=\"Required CLI and setup tools.\" enabled=\"false\" selected=\"true\">\n");
    xml.push_str("    </choice>\n");
    if include_desktop {
        xml.push_str("    <choice id=\"desktop\" title=\"Desktop App\" description=\"Zeus.app for macOS.\" selected=\"true\">\n");
        xml.push_str("    </choice>\n");
    }
    xml.push_str("    <choice id=\"services\" title=\"Services &amp; Config\" description=\"Gateway service, workspace setup, and MCP configuration.\" selected=\"true\">\n");
    xml.push_str("    </choice>\n");
    xml.push_str("    <choice id=\"extras\" title=\"Extras\" description=\"Web frontend and shell completions.\" selected=\"true\">\n");
    xml.push_str("    </choice>\n");
    xml.push('\n');

    // Component choices + pkg-ref entries
    for comp in components {
        let required = comp.identifier == "com.zeus.cli" || comp.identifier == "com.zeus.setup";
        let pkg_file = format!("{}.pkg", comp.identifier);

        xml.push_str(&format!(
            "    <choice id=\"{}\" title=\"{}\" description=\"{}\" enabled=\"{}\" selected=\"true\">\n",
            comp.identifier,
            comp.name,
            choice_description(comp.identifier),
            if required { "false" } else { "true" }
        ));
        xml.push_str(&format!("        <pkg-ref id=\"{}\" />\n", comp.identifier));
        xml.push_str("    </choice>\n");

        xml.push_str(&format!(
            "    <pkg-ref id=\"{}\" version=\"{}\" installKBytes=\"{}\">{}</pkg-ref>\n",
            comp.identifier,
            comp.version,
            estimate_install_kb(comp),
            pkg_file
        ));
    }

    xml.push('\n');

    // Install check script
    xml.push_str("    <installation-check script=\"installCheck()\" />\n");
    xml.push_str("    <script>\n");
    xml.push_str("function installCheck() {\n");
    xml.push_str(
        "    if (system.compareVersions(system.version.ProductVersion, '14.0') &lt; 0) {\n",
    );
    xml.push_str("        my.result.title = 'macOS 14.0 or later required';\n");
    xml.push_str("        my.result.message = 'Zeus requires macOS Sonoma (14.0) or later.';\n");
    xml.push_str("        my.result.type = 'Fatal';\n");
    xml.push_str("        return false;\n");
    xml.push_str("    }\n");
    xml.push_str("    return true;\n");
    xml.push_str("}\n");
    xml.push_str("    </script>\n");

    xml.push_str("</installer-gui-script>\n");

    xml
}

fn choice_description(identifier: &str) -> &'static str {
    match identifier {
        "com.zeus.cli" => "Zeus command-line interface (/usr/local/bin/zeus)",
        "com.zeus.setup" => "Zeus installer and build tool (/usr/local/bin/zeus-setup)",
        "com.zeus.desktop" => "Zeus Desktop application (/Applications/Zeus.app)",
        "com.zeus.gateway" => "Zeus gateway launchd service (auto-starts on login)",
        "com.zeus.workspace" => "Initialize ~/.zeus/ workspace with default config",
        "com.zeus.mcp" => "Configure MCP integration for Claude Code and Claude Desktop",
        "com.zeus.web" => "Zeus web frontend (/usr/local/share/zeus/web/)",
        "com.zeus.completions" => "Shell completions for bash, zsh, and fish",
        _ => "",
    }
}

fn estimate_install_kb(comp: &Component) -> u64 {
    match comp.identifier {
        "com.zeus.cli" => 15_000,     // ~15MB binary
        "com.zeus.setup" => 12_000,   // ~12MB binary
        "com.zeus.desktop" => 50_000, // ~50MB app bundle
        "com.zeus.gateway" => 5,      // tiny plist
        "com.zeus.workspace" => 10,   // config files
        "com.zeus.mcp" => 5,          // json config
        "com.zeus.web" => 5_000,      // ~5MB WASM bundle
        "com.zeus.completions" => 50, // ~50KB scripts
        _ => 100,
    }
}

fn generate_launchd_plist() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.zeus.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>__ZEUS_BIN__</string>
        <string>gateway</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>__HOME__/.zeus/logs/gateway.out.log</string>
    <key>StandardErrorPath</key>
    <string>__HOME__/.zeus/logs/gateway.err.log</string>
    <key>WorkingDirectory</key>
    <string>__HOME__</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>__HOME__</string>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin</string>
        <key>ZEUS_HOME</key>
        <string>__HOME__/.zeus</string>
    </dict>
</dict>
</plist>
"#
    .to_string()
}

/// Notarize a signed package with Apple
async fn notarize_package(
    pkg_path: &Path,
    opts: &PackageOpts,
    tx: &mpsc::Sender<ProgressEvent>,
) -> Result<()> {
    let apple_id = opts.apple_id.as_ref().unwrap();
    let team_id = opts.team_id.as_ref().unwrap();
    let pkg_str = pkg_path.to_string_lossy().to_string();

    // Submit for notarization
    tx.send(ProgressEvent::LogLine(
        "Submitting to Apple notarization service...".into(),
    ))
    .await?;

    run_command(
        tx,
        "xcrun",
        &[
            "notarytool",
            "submit",
            &pkg_str,
            "--apple-id",
            apple_id,
            "--team-id",
            team_id,
            "--keychain-profile",
            "zeus-notarize",
            "--wait",
        ],
        Path::new("/tmp"),
    )
    .await
    .context("Notarization failed. Ensure you have a keychain profile 'zeus-notarize' configured via: xcrun notarytool store-credentials zeus-notarize")?;

    // Staple the notarization ticket
    tx.send(ProgressEvent::LogLine(
        "Stapling notarization ticket...".into(),
    ))
    .await?;

    run_command(
        tx,
        "xcrun",
        &["stapler", "staple", &pkg_str],
        Path::new("/tmp"),
    )
    .await
    .context("Stapling failed")?;

    Ok(())
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if ty.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, dest_path)?;
        } else {
            std::fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}
