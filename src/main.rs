use anyhow::{Context, Result, bail};
use cargo_doc_md::ConversionOptions;
use clap::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(name = "cargo-doc-md")]
#[command(
    about = "Generate markdown documentation for Rust crates and dependencies",
    long_about = "Cargo subcommand to generate markdown documentation for Rust crates and their dependencies.\n\n\
                  Default behavior: Documents current crate + all transitive dependencies (matches cargo doc).\n\
                  Creates a master index and organizes modules into separate files for easy navigation.\n\n\
                  Usage:\n  \
                  cargo doc-md                    # Document current crate + all transitive dependencies\n  \
                  cargo doc-md --workspace        # Document all workspace members + their dependencies\n  \
                  cargo doc-md --no-deps          # Document current crate only (no dependencies)\n  \
                  cargo doc-md -p tokio           # Document tokio + all its dependencies\n  \
                  cargo doc-md -p tokio -p serde  # Document multiple packages + their dependencies\n  \
                  cargo doc-md --json file.json   # Convert existing rustdoc JSON"
)]
struct Cli {
    #[arg(
        short,
        long,
        help = "Package(s) to document with their dependencies (can be repeated)",
        conflicts_with = "workspace",
        conflicts_with = "json"
    )]
    package: Vec<String>,

    #[arg(
        short,
        long,
        default_value = "target/doc-md",
        help = "Output directory [default: target/doc-md]\n\
                Creates: target/doc-md/index.md (master index), target/doc-md/crate_name/*.md (modules)"
    )]
    output: PathBuf,

    #[arg(long, help = "Include private items in documentation")]
    include_private: bool,

    #[arg(
        long,
        help = "Convert existing rustdoc JSON file",
        conflicts_with = "package",
        conflicts_with = "workspace",
        conflicts_with = "no_deps"
    )]
    json: Option<PathBuf>,

    #[arg(
        long,
        help = "Document all workspace members (idiomatic cargo pattern)",
        conflicts_with = "package"
    )]
    workspace: bool,

    #[arg(
        long,
        help = "Don't document dependencies (matches cargo doc --no-deps)",
        conflicts_with = "json"
    )]
    no_deps: bool,

    #[arg(
        long,
        value_name = "TRIPLE",
        help = "Document for a target triple other than the host (e.g. x86_64-unknown-linux-gnu).\n\
                Lets you document platform-specific crates that don't build on the host.\n\
                Requires the target's standard library (rustup target add <triple>).",
        conflicts_with = "json"
    )]
    target: Option<String>,
}

fn main() -> Result<()> {
    // When invoked as `cargo doc-md`, cargo passes an extra "doc-md" argument
    // Skip it if present to support both `cargo doc-md` and `cargo-doc-md` invocations
    let args = std::env::args()
        .enumerate()
        .filter(|(i, arg)| !(*i == 1 && arg == "doc-md"))
        .map(|(_, arg)| arg);

    let cli = Cli::parse_from(args);

    // Verify nightly toolchain is available (unless only using --json mode)
    if cli.json.is_none() {
        check_nightly_toolchain()?;
    }

    // Validate output directory
    validate_output_directory(&cli.output)?;

    // Explicit JSON file - just convert that file
    if let Some(json_path) = cli.json.as_ref() {
        if !json_path.exists() {
            bail!("JSON file not found: {}", json_path.display());
        }
        if !json_path.is_file() {
            bail!("Path is not a file: {}", json_path.display());
        }

        // Load the JSON to extract the actual crate name from metadata
        let crate_data = cargo_doc_md::parser::load_rustdoc_json(json_path)?;
        let root_item = crate_data
            .index
            .get(&crate_data.root)
            .context("Root item not found in rustdoc JSON")?;
        let crate_name = root_item
            .name
            .as_deref()
            .context("Crate name not found in rustdoc JSON")?;

        // Remove existing crate directory to ensure clean documentation
        let crate_output_dir = cli.output.join(crate_name);
        if crate_output_dir.exists() {
            std::fs::remove_dir_all(&crate_output_dir).with_context(|| {
                format!(
                    "Failed to remove existing crate directory: {}",
                    crate_output_dir.display()
                )
            })?;
        }

        let options = ConversionOptions {
            input_path: json_path,
            output_dir: &cli.output,
            include_private: cli.include_private,
        };

        cargo_doc_md::convert_json_file(&options)?;

        // Generate master index for consistency with other modes
        generate_master_index(&cli.output, None, &[], &[crate_name.to_string()])?;

        return Ok(());
    }

    // Get cargo metadata once for all operations
    let metadata = get_cargo_metadata(cli.target.as_deref())?;

    // Workspace mode
    if cli.workspace {
        document_workspace(&metadata, &cli)?;
        return Ok(());
    }

    // Specific packages requested
    if !cli.package.is_empty() {
        document_specific_packages(&metadata, &cli)?;
        return Ok(());
    }

    // Default: document current crate + all transitive dependencies (matches cargo doc)
    if cli.no_deps {
        println!("📚 Documenting current crate only...\n");
        let current_crate = document_current_crate(&metadata, &cli)?;
        generate_master_index(&cli.output, current_crate.as_deref(), &[], &[])?;
    } else {
        println!("📚 Documenting current crate and all transitive dependencies...\n");
        let current_crate = document_current_crate(&metadata, &cli)?;
        println!();
        let documented_deps = document_all_dependencies(&metadata, &cli)?;
        generate_master_index(&cli.output, current_crate.as_deref(), &[], &documented_deps)?;
    }

    Ok(())
}

#[derive(Debug)]
struct Dependency {
    name: String,
    version: String,
}

/// Directory where rustdoc writes its JSON output. With an explicit `--target`,
/// cargo nests build artifacts under `target/<triple>/doc` instead of
/// `target/doc`, so the generated JSON must be looked up there.
fn doc_dir(target_dir: &Path, target: Option<&str>) -> PathBuf {
    match target {
        Some(triple) => target_dir.join(triple).join("doc"),
        None => target_dir.join("doc"),
    }
}

fn get_cargo_metadata(target: Option<&str>) -> Result<serde_json::Value> {
    // Filter platform-specific dependencies for the platform we're documenting:
    // the requested target if given, otherwise the host.
    let filter_triple = match target {
        Some(triple) => triple.to_string(),
        None => std::env::var("CARGO_BUILD_TARGET").or_else(|_| {
            let output = Command::new("rustc")
                .args(["-vV"])
                .output()
                .context("Failed to run rustc")?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .find(|line| line.starts_with("host:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .map(String::from)
                .context("Failed to parse host triple from rustc")
        })?,
    };

    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version=1",
            "--filter-platform",
            &filter_triple,
        ])
        .output()
        .context("Failed to run 'cargo metadata'")?;

    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    serde_json::from_slice(&output.stdout).context("Failed to parse cargo metadata")
}

fn validate_output_directory(output_dir: &Path) -> Result<()> {
    if output_dir.exists() && output_dir.is_file() {
        bail!(
            "Output path exists but is a file, not a directory: {}\n\
             Please specify a directory path or remove the file.",
            output_dir.display()
        );
    }

    Ok(())
}

fn check_nightly_toolchain() -> Result<()> {
    let output = Command::new("cargo")
        .args(["+nightly", "--version"])
        .output()
        .context("Failed to run cargo +nightly")?;

    if !output.status.success() {
        bail!(
            "Nightly toolchain not installed or not available.\n\
             This tool requires Rust nightly for unstable rustdoc features.\n\
             Install with: rustup install nightly"
        );
    }

    Ok(())
}

fn document_specific_packages(metadata: &serde_json::Value, cli: &Cli) -> Result<()> {
    println!(
        "📦 Documenting {} specific package(s) and their dependencies...",
        cli.package.len()
    );

    let packages = metadata["packages"]
        .as_array()
        .context("Missing 'packages' in metadata")?;

    let target_dir = PathBuf::from(metadata["target_directory"].as_str().unwrap_or("target"));

    // For -p mode, don't filter out workspace member dependencies
    // User explicitly requested specific packages, so document them and ALL their deps
    let workspace_member_ids: Vec<String> = Vec::new();

    let mut successful_packages = Vec::new();
    let mut failed_packages = Vec::new();
    let mut all_deps = HashMap::new();

    // Document each specified package
    for package_name in &cli.package {
        println!("\n🔨 Generating docs for '{}'...", package_name);

        // Find package in metadata
        let package = packages
            .iter()
            .find(|p| p["name"].as_str() == Some(package_name));

        let dep = if let Some(pkg) = package {
            let name = pkg["name"]
                .as_str()
                .context("Package missing 'name' field in metadata")?
                .to_string();
            let version = pkg["version"].as_str().unwrap_or("").to_string();
            Dependency { name, version }
        } else {
            Dependency {
                name: package_name.clone(),
                version: String::new(),
            }
        };

        match document_single_dependency(
            &dep,
            &cli.output,
            &target_dir,
            metadata,
            cli.include_private,
            cli.target.as_deref(),
        ) {
            Ok(true) => {
                // Successfully documented
                successful_packages.push(package_name.clone());
                println!(
                    "  ✓ {} → {}/{}/index.md",
                    package_name,
                    cli.output.display(),
                    package_name.replace("-", "_")
                );

                // Get dependencies for this package if not --no-deps
                if !cli.no_deps {
                    if let Some(pkg) = package {
                        if let Some(pkg_id) = pkg["id"].as_str() {
                            match get_all_dependencies_recursive(
                                metadata,
                                pkg_id,
                                &workspace_member_ids,
                            ) {
                                Ok(deps) => {
                                    for (name, version) in deps {
                                        if !successful_packages.contains(&name) {
                                            all_deps.insert(name, version);
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!(
                                        "  ⚠ Warning: Could not get dependencies for '{}': {}",
                                        package_name, e
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Ok(false) => {
                // Skipped (e.g., binary-only crate)
                println!("  ⊘ {} skipped", package_name);
            }
            Err(e) => {
                failed_packages.push(package_name.clone());
                println!("  ✗ Failed to document '{}': {}", package_name, e);
            }
        }
    }

    // Document dependencies if not --no-deps
    if !cli.no_deps && !all_deps.is_empty() {
        println!("\n📦 Documenting {} unique dependencies...", all_deps.len());
        let mut deps_to_document: Vec<Dependency> = all_deps
            .into_iter()
            .map(|(name, version)| Dependency { name, version })
            .collect();
        deps_to_document.sort_by(|a, b| a.name.cmp(&b.name));

        let (successful_deps, failed_deps) = try_document_dependencies(
            &deps_to_document,
            &cli.output,
            &target_dir,
            metadata,
            cli.include_private,
            cli.target.as_deref(),
        );

        print_documentation_summary(&successful_deps, &failed_deps);

        generate_master_index(&cli.output, None, &successful_packages, &successful_deps)?;
    } else {
        println!("\n📊 Summary:");
        println!("  ✓ Packages documented: {}", successful_packages.len());
        if !failed_packages.is_empty() {
            println!(
                "  ✗ Failed: {} ({})",
                failed_packages.len(),
                failed_packages.join(", ")
            );
        }

        generate_master_index(&cli.output, None, &[], &successful_packages)?;
    }

    Ok(())
}

/// Get the library target name from a package (may differ from package name)
fn get_lib_target_name(package: &serde_json::Value) -> Option<String> {
    package["targets"]
        .as_array()?
        .iter()
        .find(|target| {
            target["kind"]
                .as_array()
                .map(|kinds| kinds.iter().any(|k| k.as_str() == Some("lib")))
                .unwrap_or(false)
        })
        .and_then(|target| target["name"].as_str())
        .map(String::from)
}

fn document_current_crate(metadata: &serde_json::Value, cli: &Cli) -> Result<Option<String>> {
    println!("🔨 Generating rustdoc JSON for current crate...");

    // Run cargo rustdoc to generate JSON. `--target` (when set) is a cargo-level
    // flag and must come before the `--` that separates the rustdoc flags.
    let mut args = vec!["+nightly", "rustdoc", "--lib"];

    if let Some(target) = cli.target.as_deref() {
        args.push("--target");
        args.push(target);
    }

    args.extend(["--", "--output-format=json", "-Z", "unstable-options"]);

    if cli.include_private {
        args.push("--document-private-items");
    }

    let output = Command::new("cargo")
        .args(&args)
        .output()
        .context("Failed to run cargo rustdoc")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        if stderr.contains("no library targets found") {
            println!(
                "⚠ No library target found in current crate, skipping current crate documentation"
            );
            return Ok(None);
        }

        bail!("cargo rustdoc failed:\n{}", stderr);
    }

    let packages = metadata["packages"]
        .as_array()
        .context("Missing 'packages' in metadata")?;

    // Get the root package name using resolve.root
    let Some(root_id) = metadata["resolve"]["root"].as_str() else {
        bail!(
            "Cannot document current crate from a virtual workspace root.\n\
             Virtual workspaces have no root package.\n\
             Use: cargo doc-md --workspace  (to document all workspace members)\n\
             Or:  cargo doc-md -p <package>  (to document a specific package)"
        );
    };

    let root_package = packages
        .iter()
        .find(|p| p["id"].as_str() == Some(root_id))
        .context("Root package not found in packages list")?;

    let crate_name = root_package["name"]
        .as_str()
        .context("Root package missing name")?
        .to_string();

    // Get the library target name (may differ from package name)
    let lib_target_name =
        get_lib_target_name(root_package).unwrap_or_else(|| crate_name.replace("-", "_"));

    // Find the generated JSON file
    let target_dir = PathBuf::from(metadata["target_directory"].as_str().unwrap_or("target"));
    let json_path =
        doc_dir(&target_dir, cli.target.as_deref()).join(format!("{}.json", lib_target_name));

    if !json_path.exists() {
        bail!("Generated JSON file not found at {}", json_path.display());
    }

    println!("✓ JSON generated successfully");
    println!("🔄 Converting to markdown...");

    // Remove existing crate directory to ensure clean documentation
    let crate_output_dir = cli.output.join(&lib_target_name);
    if crate_output_dir.exists() {
        std::fs::remove_dir_all(&crate_output_dir).with_context(|| {
            format!(
                "Failed to remove existing crate directory: {}",
                crate_output_dir.display()
            )
        })?;
    }

    // Convert to markdown
    let options = ConversionOptions {
        input_path: &json_path,
        output_dir: &cli.output,
        include_private: cli.include_private,
    };

    cargo_doc_md::convert_json_file(&options)?;

    println!(
        "✓ Current crate documented: {}/{}/index.md",
        cli.output.display(),
        crate_name.replace("-", "_")
    );

    Ok(Some(crate_name))
}

fn try_document_dependencies(
    deps_to_document: &[Dependency],
    output_dir: &Path,
    target_dir: &Path,
    metadata: &serde_json::Value,
    include_private: bool,
    target: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let mut successful = Vec::new();
    let mut failed = Vec::new();

    for dep in deps_to_document {
        match document_single_dependency(
            dep,
            output_dir,
            target_dir,
            metadata,
            include_private,
            target,
        ) {
            Ok(true) => {
                // Successfully documented
                successful.push(dep.name.clone());
                println!(
                    "  ✓ {} → {}/{}/index.md",
                    dep.name,
                    output_dir.display(),
                    dep.name.replace("-", "_")
                );
            }
            Ok(false) => {
                // Skipped (e.g., binary-only crate) - not added to successful or failed
            }
            Err(e) => {
                failed.push(dep.name.clone());
                println!("  ✗ {} - {}", dep.name, e);
            }
        }
    }

    (successful, failed)
}

fn print_documentation_summary(successful: &[String], failed: &[String]) {
    println!("\n📊 Summary:");
    println!("  ✓ Successful: {}", successful.len());
    if !failed.is_empty() {
        println!("  ✗ Failed: {} ({})", failed.len(), failed.join(", "));
    }
}

fn document_all_dependencies(metadata: &serde_json::Value, cli: &Cli) -> Result<Vec<String>> {
    let deps_to_document = get_all_dependencies(metadata)?;

    if deps_to_document.is_empty() {
        println!("No dependencies found");
        return Ok(Vec::new());
    }

    let target_dir = PathBuf::from(metadata["target_directory"].as_str().unwrap_or("target"));

    println!("📦 Documenting {} dependencies...", deps_to_document.len());

    let (successful, failed) = try_document_dependencies(
        &deps_to_document,
        &cli.output,
        &target_dir,
        metadata,
        cli.include_private,
        cli.target.as_deref(),
    );

    print_documentation_summary(&successful, &failed);

    Ok(successful)
}

fn document_workspace(metadata: &serde_json::Value, cli: &Cli) -> Result<()> {
    let workspace_members = get_workspace_members(metadata)?;

    println!(
        "📚 Documenting {} workspace member(s){}...\n",
        workspace_members.len(),
        if cli.no_deps {
            " (without dependencies)"
        } else {
            " and their dependencies"
        }
    );

    let target_dir = PathBuf::from(metadata["target_directory"].as_str().unwrap_or("target"));

    let workspace_member_ids: Vec<String> = metadata["workspace_members"]
        .as_array()
        .map(|members| {
            members
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let workspace_member_names: std::collections::HashSet<String> =
        workspace_members.iter().map(|m| m.name.clone()).collect();

    let mut successful_members = Vec::new();
    let mut failed_members = Vec::new();
    let mut all_deps: HashMap<String, String> = HashMap::new();

    for member in &workspace_members {
        println!(
            "🔨 Generating docs for workspace member '{}'...",
            member.name
        );

        match document_single_dependency(
            member,
            &cli.output,
            &target_dir,
            metadata,
            cli.include_private,
            cli.target.as_deref(),
        ) {
            Ok(true) => {
                // Successfully documented
                successful_members.push(member.name.clone());
                println!(
                    "  ✓ {} → {}/{}/index.md",
                    member.name,
                    cli.output.display(),
                    member.name.replace("-", "_")
                );

                if !cli.no_deps {
                    match get_package_id(metadata, &member.name, &member.version) {
                        Ok(member_id) => {
                            match get_all_dependencies_recursive(
                                metadata,
                                &member_id,
                                &workspace_member_ids,
                            ) {
                                Ok(member_deps) => {
                                    for (name, version) in member_deps {
                                        if !workspace_member_names.contains(&name) {
                                            all_deps.insert(name, version);
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!(
                                        "  ⚠ Warning: Could not get dependencies for '{}': {}",
                                        member.name, e
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            println!(
                                "  ⚠ Warning: Could not find package ID for '{}': {}",
                                member.name, e
                            );
                        }
                    }
                }
            }
            Ok(false) => {
                // Skipped (e.g., binary-only crate)
                println!("  ⊘ {} skipped", member.name);
            }
            Err(e) => {
                failed_members.push(member.name.clone());
                println!("  ✗ Failed to document '{}': {}", member.name, e);
            }
        }
    }

    if !cli.no_deps && !all_deps.is_empty() {
        println!(
            "\n📦 Documenting {} unique external dependencies...",
            all_deps.len()
        );
        let mut deps_to_document: Vec<Dependency> = all_deps
            .into_iter()
            .map(|(name, version)| Dependency { name, version })
            .collect();
        deps_to_document.sort_by(|a, b| a.name.cmp(&b.name));

        let (successful_deps, failed_deps) = try_document_dependencies(
            &deps_to_document,
            &cli.output,
            &target_dir,
            metadata,
            cli.include_private,
            cli.target.as_deref(),
        );

        print_documentation_summary(&successful_deps, &failed_deps);

        generate_master_index(&cli.output, None, &successful_members, &successful_deps)?;
    } else {
        println!("\n📊 Summary:");
        println!(
            "  ✓ Workspace members documented: {}",
            successful_members.len()
        );
        if !failed_members.is_empty() {
            println!(
                "  ✗ Failed: {} ({})",
                failed_members.len(),
                failed_members.join(", ")
            );
        }

        generate_master_index(&cli.output, None, &successful_members, &[])?;
    }

    Ok(())
}

fn get_package_id(metadata: &serde_json::Value, name: &str, version: &str) -> Result<String> {
    let packages = metadata["packages"]
        .as_array()
        .context("Missing 'packages' in metadata")?;

    for pkg in packages {
        if pkg["name"].as_str() == Some(name) && pkg["version"].as_str() == Some(version) {
            return pkg["id"]
                .as_str()
                .map(String::from)
                .context("Package ID not found");
        }
    }

    bail!("Package {} {} not found in metadata", name, version)
}

fn build_normal_dependency_graph(
    metadata: &serde_json::Value,
) -> Result<HashMap<String, Vec<String>>> {
    use std::collections::HashSet;

    let packages = metadata["packages"]
        .as_array()
        .context("Missing 'packages' in metadata")?;

    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .context("Missing 'nodes' in resolve")?;

    let mut normal_dep_graph: HashMap<String, Vec<String>> = HashMap::new();

    // For each package, build a map of package_id -> Set of normal dependency names
    let mut pkg_normal_deps: HashMap<String, HashSet<String>> = HashMap::new();
    for pkg in packages {
        let Some(pkg_id) = pkg["id"].as_str() else {
            continue; // Skip packages without IDs
        };
        let mut normal_deps = HashSet::new();

        if let Some(deps) = pkg["dependencies"].as_array() {
            for dep in deps {
                // Only include normal dependencies (kind is null or not present)
                // Platform filtering is handled by cargo metadata --filter-platform
                if dep["kind"].is_null() {
                    if let Some(dep_name) = dep["name"].as_str() {
                        normal_deps.insert(dep_name.to_string());
                    }
                }
            }
        }
        pkg_normal_deps.insert(pkg_id.to_string(), normal_deps);
    }

    // Build the filtered dependency graph
    for node in nodes {
        let Some(node_id) = node["id"].as_str() else {
            continue; // Skip nodes without IDs
        };
        let node_pkg = packages.iter().find(|p| p["id"].as_str() == Some(node_id));
        let normal_deps = pkg_normal_deps.get(node_id);

        if let (Some(_pkg), Some(normal_dep_names)) = (node_pkg, normal_deps) {
            let mut filtered_deps = Vec::new();

            if let Some(dep_ids) = node["dependencies"].as_array() {
                for dep_id in dep_ids {
                    if let Some(dep_id_str) = dep_id.as_str() {
                        // Check if this dependency is in the normal deps list
                        if let Some(dep_pkg) = packages
                            .iter()
                            .find(|p| p["id"].as_str() == Some(dep_id_str))
                        {
                            if let Some(dep_name) = dep_pkg["name"].as_str() {
                                if normal_dep_names.contains(dep_name) {
                                    filtered_deps.push(dep_id_str.to_string());
                                }
                            }
                        }
                    }
                }
            }

            normal_dep_graph.insert(node_id.to_string(), filtered_deps);
        }
    }

    Ok(normal_dep_graph)
}

fn get_all_dependencies_recursive(
    metadata: &serde_json::Value,
    package_id: &str,
    workspace_member_ids: &[String],
) -> Result<HashMap<String, String>> {
    use std::collections::HashSet;

    let packages = metadata["packages"]
        .as_array()
        .context("Missing 'packages' in metadata")?;

    let normal_dep_graph = build_normal_dependency_graph(metadata)?;

    let mut all_deps = HashMap::new();
    let mut visited = HashSet::new();
    let mut to_visit = vec![package_id.to_string()];

    while let Some(current_id) = to_visit.pop() {
        if visited.contains(&current_id) {
            continue;
        }
        visited.insert(current_id.clone());

        if let Some(dep_ids) = normal_dep_graph.get(&current_id) {
            for dep_id in dep_ids {
                // Skip workspace members
                if workspace_member_ids.contains(dep_id) {
                    continue;
                }

                // Add to visit queue for recursive traversal
                if !visited.contains(dep_id) {
                    to_visit.push(dep_id.clone());
                }

                // Add to result if not already there
                if let Some(pkg) = packages
                    .iter()
                    .find(|p| p["id"].as_str() == Some(dep_id.as_str()))
                {
                    if let (Some(name), Some(version)) =
                        (pkg["name"].as_str(), pkg["version"].as_str())
                    {
                        // HashMap automatically deduplicates by name (matching cargo doc behavior)
                        all_deps.insert(name.to_string(), version.to_string());
                    }
                }
            }
        }
    }

    Ok(all_deps)
}

fn get_all_dependencies(metadata: &serde_json::Value) -> Result<Vec<Dependency>> {
    let resolve = &metadata["resolve"];
    let Some(root) = resolve["root"].as_str() else {
        bail!(
            "Cannot document dependencies from a virtual workspace root.\n\
             Virtual workspaces have no root package.\n\
             Use: cargo doc-md --workspace  (to document all workspace members)\n\
             Or:  cargo doc-md -p <package>  (to document a specific package)"
        );
    };

    let workspace_member_ids: Vec<String> = metadata["workspace_members"]
        .as_array()
        .map(|members| {
            members
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let deps_map = get_all_dependencies_recursive(metadata, root, &workspace_member_ids)?;

    let mut deps: Vec<Dependency> = deps_map
        .into_iter()
        .map(|(name, version)| Dependency { name, version })
        .collect();

    deps.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(deps)
}

fn get_workspace_members(metadata: &serde_json::Value) -> Result<Vec<Dependency>> {
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .context("Missing 'workspace_members' in metadata")?;

    if workspace_members.is_empty() {
        bail!(
            "Not in a workspace or workspace has no members.\n\
             The --workspace flag requires a Cargo workspace.\n\
             For single-crate projects, use: cargo doc-md (without --workspace)"
        );
    }

    let packages = metadata["packages"]
        .as_array()
        .context("Missing 'packages' in metadata")?;

    let mut members = Vec::new();
    for member_id in workspace_members {
        let member_id_str = member_id.as_str().context("Invalid workspace member ID")?;

        if let Some(pkg) = packages
            .iter()
            .find(|p| p["id"].as_str() == Some(member_id_str))
        {
            if let (Some(name), Some(version)) = (pkg["name"].as_str(), pkg["version"].as_str()) {
                members.push(Dependency {
                    name: name.to_string(),
                    version: version.to_string(),
                });
            }
        }
    }

    members.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(members)
}

/// Returns Ok(true) if documented, Ok(false) if skipped (e.g., binary-only crate), Err on failure
fn document_single_dependency(
    dep: &Dependency,
    output_base: &Path,
    target_dir: &Path,
    metadata: &serde_json::Value,
    include_private: bool,
    target: Option<&str>,
) -> Result<bool> {
    // Build the package specification
    // If we have a version, use name@version to disambiguate multiple versions
    let package_spec = if dep.version.is_empty() {
        dep.name.clone()
    } else {
        format!("{}@{}", dep.name, dep.version)
    };

    // Generate rustdoc JSON for the dependency. `--target` (when set) is a
    // cargo-level flag and must come before the `--` that separates the
    // rustdoc flags.
    let mut args = vec!["+nightly", "rustdoc", "-p", &package_spec, "--lib"];

    if let Some(target) = target {
        args.push("--target");
        args.push(target);
    }

    args.extend(["--", "--output-format=json", "-Z", "unstable-options"]);

    if include_private {
        args.push("--document-private-items");
    }

    let output = Command::new("cargo")
        .args(&args)
        .output()
        .context("Failed to run cargo rustdoc")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check for known non-error cases
        if stderr.contains("no library targets found") {
            println!("  ⚠ No library target found (binary-only crate), skipping documentation");
            return Ok(false); // Skipped, not an error
        }
        // Show first few error lines
        let error_lines: Vec<&str> = stderr
            .lines()
            .filter(|line| line.contains("error") || line.contains("failed"))
            .take(2)
            .collect();
        if !error_lines.is_empty() {
            bail!(
                "Failed to build '{}':\n{}\n\nRun 'cargo build -p {}' for full details",
                dep.name,
                error_lines.join("\n"),
                package_spec
            );
        }
        bail!(
            "Failed to build '{}' (exit code: {})\nRun 'cargo build -p {}' for details",
            dep.name,
            output.status,
            package_spec
        );
    }

    // Find the package in metadata to get lib target name
    let packages = metadata["packages"]
        .as_array()
        .context("Missing 'packages' in metadata")?;

    let package = packages.iter().find(|p| {
        if let (Some(pkg_name), Some(pkg_version)) = (p["name"].as_str(), p["version"].as_str()) {
            pkg_name == dep.name && (dep.version.is_empty() || pkg_version == dep.version)
        } else {
            false
        }
    });

    // Get the library target name (may differ from package name)
    let lib_target_name = package
        .and_then(get_lib_target_name)
        .unwrap_or_else(|| dep.name.replace("-", "_"));

    // Find the generated JSON file
    let json_path = doc_dir(target_dir, target).join(format!("{}.json", lib_target_name));

    if !json_path.exists() {
        bail!("Generated JSON file not found at {}", json_path.display());
    }

    // Remove existing crate directory to ensure clean documentation
    let crate_output_dir = output_base.join(&lib_target_name);
    if crate_output_dir.exists() {
        std::fs::remove_dir_all(&crate_output_dir).with_context(|| {
            format!(
                "Failed to remove existing crate directory: {}",
                crate_output_dir.display()
            )
        })?;
    }

    // Convert to markdown directly in output directory
    // The converter will create a subdirectory with the crate name
    let options = ConversionOptions {
        input_path: &json_path,
        output_dir: output_base,
        include_private,
    };

    cargo_doc_md::convert_json_file(&options)?;

    Ok(true) // Successfully documented
}

fn generate_master_index(
    output_dir: &Path,
    current_crate: Option<&str>,
    workspace_members: &[String],
    dependencies: &[String],
) -> Result<()> {
    use std::fs;

    let mut content = String::new();

    content.push_str("# Documentation Index\n\n");
    content.push_str("Generated markdown documentation for this project.\n\n");

    // Current crate section
    if let Some(crate_name) = current_crate {
        content.push_str("## Current Crate\n\n");
        content.push_str(&format!(
            "- [`{}`]({}/index.md)\n\n",
            crate_name,
            crate_name.replace("-", "_")
        ));
    }

    // Workspace members section
    if !workspace_members.is_empty() {
        content.push_str(&format!(
            "## Workspace Members ({})\n\n",
            workspace_members.len()
        ));

        for member in workspace_members {
            let member_path = format!("{}/index.md", member.replace("-", "_"));
            content.push_str(&format!("- [`{}`]({})\n", member, member_path));
        }
        content.push('\n');
    }

    // Dependencies section
    if !dependencies.is_empty() {
        content.push_str(&format!("## Dependencies ({})\n\n", dependencies.len()));

        for dep in dependencies {
            let dep_path = format!("{}/index.md", dep.replace("-", "_"));
            content.push_str(&format!("- [`{}`]({})\n", dep, dep_path));
        }
        content.push('\n');
    }

    content.push_str("---\n\n");
    content
        .push_str("Generated with [cargo-doc-md](https://github.com/Crazytieguy/cargo-doc-md)\n");

    // Ensure output directory exists
    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    let index_path = output_dir.join("index.md");
    fs::write(&index_path, content)
        .with_context(|| format!("Failed to write master index: {}", index_path.display()))?;

    println!("\n✓ Master index: {}", index_path.display());

    Ok(())
}
