use std::fs;
use std::path::{Path, PathBuf};

use agena_plugin_marketplace::{
    AGENA_MARKETPLACE_FILENAME, AGENA_MARKETPLACE_PROJECT_FILENAME,
    AGENA_RELEASE_MANIFEST_FILENAME, AddMarketplaceReleaseRequest, AssembleReleaseRequest,
    BuildMarketplaceRequest, MarketplaceProjectManifest, PackagePluginRequest,
    PluginProjectManifest, PluginReleaseManifest, PluginReleaseSource, PluginTemplateKind,
    RegistryIndex, ScaffoldMarketplaceRequest, ScaffoldPluginRequest, add_marketplace_release,
    assemble_release, build_marketplace, current_target_triple, generate_plugin_lockfile,
    package_plugin, scaffold_marketplace, scaffold_plugin,
};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "agena-plugin", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Args)]
struct MarketplaceBuildArgs {
    releases: PathBuf,
    #[arg(long, default_value = AGENA_MARKETPLACE_PROJECT_FILENAME)]
    project: PathBuf,
    #[arg(long, default_value = AGENA_MARKETPLACE_FILENAME)]
    output: PathBuf,
    #[arg(long, default_value_t = false)]
    github_only: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scaffold a standalone plugin repository.
    Init(InitArgs),
    /// Validate an Agena plugin project, release manifest, or marketplace index.
    Validate(ValidateArgs),
    /// Package a compiled plugin artifact for one target.
    Package(PackageArgs),
    /// Assemble platform fragments into a release manifest.
    Release(ReleaseOperation),
    /// Scaffold and maintain a GitHub-first marketplace repository.
    Marketplace(MarketplaceOperation),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TemplateKind {
    Stdio,
    Cdylib,
}

#[derive(Debug, Args)]
struct InitArgs {
    path: PathBuf,
    #[arg(long)]
    id: String,
    #[arg(long)]
    crate_name: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long, default_value = "An Agena plugin.")]
    description: String,
    #[arg(long, default_value = "Agena Plugin Authors")]
    author: String,
    #[arg(long)]
    repository: Option<String>,
    #[arg(long, default_value_t = TemplateKind::Stdio, value_enum)]
    kind: TemplateKind,
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[derive(Debug, Args)]
struct ValidateArgs {
    path: PathBuf,
}

#[derive(Debug, Args)]
struct PackageArgs {
    #[arg(long, default_value = "agena-plugin.toml")]
    manifest: PathBuf,
    #[arg(long)]
    artifact: PathBuf,
    #[arg(long)]
    target: Option<String>,
    #[arg(long, default_value = "dist")]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct ReleaseOperation {
    #[command(subcommand)]
    command: ReleaseCommand,
}

#[derive(Debug, Subcommand)]
enum ReleaseCommand {
    Assemble(ReleaseAssembleArgs),
}

#[derive(Debug, Args)]
struct ReleaseAssembleArgs {
    fragments: PathBuf,
    #[arg(long, default_value = "dist/release")]
    output: PathBuf,
    #[arg(long)]
    base_url: String,
    #[arg(long)]
    expected_version: Option<String>,
    #[arg(long, requires = "github_tag", requires = "github_commit")]
    github_repository: Option<String>,
    #[arg(long, requires = "github_repository", requires = "github_commit")]
    github_tag: Option<String>,
    #[arg(long, requires = "github_repository", requires = "github_tag")]
    github_commit: Option<String>,
    #[arg(long, requires = "github_repository")]
    github_workflow_run_url: Option<String>,
}

#[derive(Debug, Args)]
struct MarketplaceOperation {
    #[command(subcommand)]
    command: MarketplaceCommand,
}

#[derive(Debug, Subcommand)]
enum MarketplaceCommand {
    /// Scaffold a GitHub-first marketplace repository.
    Init(MarketplaceInitArgs),
    /// Add one immutable published release and rebuild the index.
    Add(MarketplaceAddArgs),
    /// Rebuild a deterministic index from a directory of release manifests.
    Build(MarketplaceBuildArgs),
    /// Validate a marketplace index.
    Validate(MarketplaceValidateArgs),
}

#[derive(Debug, Args)]
struct MarketplaceInitArgs {
    path: PathBuf,
    #[arg(long)]
    name: String,
    #[arg(long)]
    description: String,
    #[arg(long)]
    repository: String,
    #[arg(long, default_value = "Agena Marketplace Maintainers")]
    owner: String,
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[derive(Debug, Args)]
struct MarketplaceAddArgs {
    release: PathBuf,
    #[arg(long, default_value = "releases")]
    releases: PathBuf,
    #[arg(long, default_value = AGENA_MARKETPLACE_PROJECT_FILENAME)]
    project: PathBuf,
    #[arg(long, default_value = AGENA_MARKETPLACE_FILENAME)]
    index: PathBuf,
    #[arg(long, default_value_t = false)]
    github_only: bool,
}

#[derive(Debug, Args)]
struct MarketplaceValidateArgs {
    #[arg(default_value = AGENA_MARKETPLACE_FILENAME)]
    index: PathBuf,
    #[arg(long, default_value_t = false)]
    github_only: bool,
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Init(args) => {
            let short_name = args
                .id
                .split_once('.')
                .map(|(_, name)| name)
                .unwrap_or(args.id.as_str())
                .to_string();
            let crate_name = args
                .crate_name
                .unwrap_or_else(|| format!("agena-plugin-{}", short_name.replace('_', "-")));
            let display_name = args.name.unwrap_or_else(|| humanize(&short_name));
            scaffold_plugin(ScaffoldPluginRequest {
                destination: args.path.clone(),
                plugin_id: args.id,
                crate_name,
                display_name,
                description: args.description,
                author: args.author,
                repository: args.repository,
                kind: match args.kind {
                    TemplateKind::Stdio => PluginTemplateKind::Stdio,
                    TemplateKind::Cdylib => PluginTemplateKind::Cdylib,
                },
                force: args.force,
            })?;
            generate_plugin_lockfile(&args.path)?;
            println!("created {}", args.path.display());
        }
        Command::Validate(args) => {
            let path = resolve_validation_path(&args.path)?;
            let kind = validate_path(&path)?;
            println!("valid {kind}: {}", path.display());
        }
        Command::Package(args) => {
            let output = package_plugin(PackagePluginRequest {
                manifest_path: args.manifest,
                artifact_path: args.artifact,
                target: args
                    .target
                    .unwrap_or_else(|| current_target_triple().to_string()),
                output_dir: args.output,
            })?;
            println!("archive={}", output.archive_path.display());
            println!("fragment={}", output.fragment_path.display());
            println!("sha256={}", output.sha256);
        }
        Command::Release(operation) => match operation.command {
            ReleaseCommand::Assemble(args) => {
                let source = args
                    .github_repository
                    .map(|repository| PluginReleaseSource {
                        repository,
                        tag: args.github_tag.expect("clap requires github_tag"),
                        commit: args.github_commit.expect("clap requires github_commit"),
                        workflow_run_url: args.github_workflow_run_url,
                    });
                let output = assemble_release(AssembleReleaseRequest {
                    fragments_dir: args.fragments,
                    output_dir: args.output,
                    base_url: args.base_url,
                    expected_version: args.expected_version,
                    source,
                })?;
                println!("manifest={}", output.release_manifest_path.display());
                println!("artifacts={}", output.artifact_count);
            }
        },
        Command::Marketplace(operation) => match operation.command {
            MarketplaceCommand::Init(args) => {
                scaffold_marketplace(ScaffoldMarketplaceRequest {
                    destination: args.path.clone(),
                    name: args.name,
                    description: args.description,
                    repository: args.repository,
                    owner_name: args.owner,
                    force: args.force,
                })?;
                println!("created marketplace repository {}", args.path.display());
            }
            MarketplaceCommand::Add(args) => {
                let output = add_marketplace_release(AddMarketplaceReleaseRequest {
                    release_path: args.release,
                    releases_dir: args.releases,
                    project_path: Some(args.project),
                    index_path: args.index,
                    github_only: args.github_only,
                })?;
                println!(
                    "stored={} index={} plugins={} releases={} unchanged={}",
                    output.stored_release_path.display(),
                    output.index_path.display(),
                    output.plugin_count,
                    output.release_count,
                    output.already_present
                );
            }
            MarketplaceCommand::Build(args) => {
                let output = build_marketplace(BuildMarketplaceRequest {
                    releases_dir: args.releases,
                    project_path: Some(args.project),
                    output_path: args.output,
                    github_only: args.github_only,
                })?;
                println!(
                    "built {} plugin(s), {} release(s) into {}",
                    output.plugin_count,
                    output.release_count,
                    output.output_path.display()
                );
            }
            MarketplaceCommand::Validate(args) => {
                let index: RegistryIndex = serde_json::from_slice(&fs::read(&args.index)?)?;
                if args.github_only {
                    index.validate_github_distribution()?;
                } else {
                    index.validate()?;
                }
                println!(
                    "valid marketplace: {} plugin(s) in {}",
                    index.plugins.len(),
                    args.index.display()
                );
            }
        },
    }
    Ok(())
}

fn resolve_validation_path(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    for candidate in [
        path.join(agena_plugin_marketplace::AGENA_PROJECT_MANIFEST_FILENAME),
        path.join(AGENA_MARKETPLACE_PROJECT_FILENAME),
        path.join(AGENA_RELEASE_MANIFEST_FILENAME),
        path.join(AGENA_MARKETPLACE_FILENAME),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("no Agena plugin manifest found at {}", path.display()).into())
}

fn validate_path(path: &Path) -> Result<&'static str, Box<dyn std::error::Error>> {
    if path
        .file_name()
        .is_some_and(|name| name == agena_plugin_marketplace::AGENA_PROJECT_MANIFEST_FILENAME)
    {
        PluginProjectManifest::load(path)?;
        return Ok("plugin project");
    }
    if path
        .file_name()
        .is_some_and(|name| name == AGENA_MARKETPLACE_PROJECT_FILENAME)
    {
        MarketplaceProjectManifest::load(path)?;
        return Ok("marketplace project");
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value.get("plugins").is_some() {
        let index: RegistryIndex = serde_json::from_value(value)?;
        index.validate()?;
        return Ok("marketplace index");
    }
    if value.get("artifacts").is_some() {
        let release: PluginReleaseManifest = serde_json::from_value(value)?;
        release.validate()?;
        return Ok("plugin release");
    }
    Err("expected agena-plugin.toml, agena-marketplace.toml, agena-plugin-release.json, or agena-marketplace.json".into())
}

fn humanize(value: &str) -> String {
    value
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
