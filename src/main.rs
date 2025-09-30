use std::io::Write;
use clap::{Arg, ArgMatches, Command};
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command as ShellCommand,
};
use log::{info, debug, error};

use walkdir::WalkDir;
use globset::{Glob, GlobSetBuilder};

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    #[serde(skip)]
    name: String,
    about: String,
    included_paths: Vec<String>,
    excluded_paths: Vec<String>,
    included_types: Vec<String>,
    excluded_types: Vec<String>,
}

type Configs = HashMap<String, Config>;

#[derive(Debug)]
struct Kat {
    configs: Configs,
}

impl Config {
    /// Construct a Config from clap’s ArgMatches (for “ptns” cases)
    fn from_matches(name: &str, about: &str, sub_m: &ArgMatches) -> Config {
        let included_paths = if let Some(vals) = sub_m.get_many::<String>("included-paths") {
            vals.map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        };

        let excluded_paths = if let Some(vals) = sub_m.get_many::<String>("excluded-paths") {
            vals.map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        };

        let included_types = if let Some(vals) = sub_m.get_many::<String>("included-types") {
            vals.map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        };

        let excluded_types = if let Some(vals) = sub_m.get_many::<String>("excluded-types") {
            vals.map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        };

        Config {
            name: name.to_string(),
            about: about.to_string(),
            included_paths,
            excluded_paths,
            included_types,
            excluded_types,
        }
    }
}

impl Kat {
    fn new(config_dir: PathBuf) -> Result<Self> {
        info!("Initializing Kat with config directory: {}", config_dir.display());
        let configs = Kat::load_configs(&config_dir)?;
        Ok(Self { configs })
    }

    fn load_configs(config_dir: &Path) -> Result<Configs> {
        if !config_dir.exists() {
            error!("Config directory not found: {}", config_dir.display());
            return Err(eyre!("Config directory not found: {}", config_dir.display()));
        }

        let mut configs = Configs::new();

        for entry in fs::read_dir(config_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(extension) = path.extension() {
                    if extension == "yml" || extension == "yaml" {
                        info!("Loading config file: {}", path.display());
                        let config_content = fs::read_to_string(&path)?;
                        let mut config: Config = serde_yaml::from_str(&config_content)?;

                        if let Some(file_stem) = path.file_stem() {
                            if let Some(name_str) = file_stem.to_str() {
                                config.name = name_str.to_string();
                                configs.insert(name_str.to_string(), config);
                                debug!("Added config: {}", name_str);
                            }
                        }
                    }
                }
            }
        }

        Ok(configs)
    }

    fn config_to_command(config: &Config) -> Command {
        let cmd = Command::new(&config.name)
            .about(&config.about)
            .arg(
                Arg::new("path")
                    .short('p')
                    .long("path")
                    .value_name("PATH")
                    .default_value(".")
                    .help("Path to start from (file or directory)")
                    .required(false),
            );
        Kat::add_common_args(cmd, Some(config))
    }

    fn create_ptns_command() -> Command {
        let cmd = Command::new("ptns")
            .about("Use arbitrary glob patterns instead of a YAML config")
            .arg(
                Arg::new("path")
                    .short('p')
                    .long("path")
                    .value_name("PATH")
                    .default_value(".")
                    .help("Path to start from (file or directory)")
                    .required(false),
            );
        Kat::add_common_args(cmd, None)
    }

    /// Build the top‐level `kat` command, register all dynamic subcommands first,
    /// then append the "ptns" subcommand last.
    pub fn configs_to_command(configs: &Configs) -> Command {
        let mut command = Command::new("kat")
            .about("Concatenate files with metadata")
            .version(env!("GIT_DESCRIBE"))
            .arg(
                Arg::new("show-patterns")
                    .short('P')
                    .long("show-patterns")
                    .help("Show the resulting include and exclude patterns")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("show-paths")
                    .short('p')
                    .long("show-paths")
                    .help("Show the resulting paths only")
                    .action(clap::ArgAction::SetTrue),
            );

        // Register all YAML-based subcommands:
        for config in configs.values() {
            let subcommand = Kat::config_to_command(config);
            command = command.subcommand(subcommand);
        }

        // Append the ad-hoc "ptns" command:
        let ptns_cmd = Kat::create_ptns_command();
        command = command.subcommand(ptns_cmd);

        command
    }

    /// Add “included-paths”, “excluded-paths”, “included-types”, and “excluded-types”
    /// arguments to a given Command. When `config` is `Some(cfg)`, set default_values
    /// from `cfg`. Otherwise leave defaults empty, requiring the user to supply at least one.
    fn add_common_args(mut cmd: Command, config: Option<&Config>) -> Command {
        let mut inc_paths = Arg::new("included-paths")
            .short('i')
            .long("included-paths")
            .help("Included paths")
            .num_args(1..)
            .value_delimiter(' ');
        if let Some(cfg) = config {
            inc_paths = inc_paths.default_values(&cfg.included_paths);
        }
        cmd = cmd.arg(inc_paths);

        let mut exc_paths = Arg::new("excluded-paths")
            .short('x')
            .long("excluded-paths")
            .help("Excluded paths")
            .num_args(1..)
            .value_delimiter(' ');
        if let Some(cfg) = config {
            exc_paths = exc_paths.default_values(&cfg.excluded_paths);
        }
        cmd = cmd.arg(exc_paths);

        let mut inc_types = Arg::new("included-types")
            .short('I')
            .long("included-types")
            .help("Included types")
            .num_args(1..)
            .value_delimiter(' ');
        if let Some(cfg) = config {
            inc_types = inc_types.default_values(&cfg.included_types);
        }
        cmd = cmd.arg(inc_types);

        let mut exc_types = Arg::new("excluded-types")
            .short('X')
            .long("excluded-types")
            .help("Excluded types")
            .num_args(1..)
            .value_delimiter(' ');
        if let Some(cfg) = config {
            exc_types = exc_types.default_values(&cfg.excluded_types);
        }
        cmd = cmd.arg(exc_types);

        cmd
    }

    pub fn parse(configs: &Configs, args: &[String]) -> Result<ArgMatches> {
        let kat_command = Kat::configs_to_command(configs);
        match kat_command.try_get_matches_from(args) {
            Ok(matches) => Ok(matches),
            Err(err) if err.use_stderr() => Err(eyre!(err.to_string())),
            Err(err) => {
                err.print()?;
                std::process::exit(0);
            }
        }
    }

    pub fn run_subcommand(
        &self,
        subcommand: &str,
        path_override: Option<PathBuf>,
        show_patterns: bool,
        show_paths: bool,
    ) -> Result<Vec<PathBuf>> {
        let config = self
            .configs
            .get(subcommand)
            .ok_or_else(|| eyre!("Config for '{}' not found", subcommand))?;

        let start_path = path_override
            .map(fs::canonicalize)
            .transpose()?
            .unwrap_or_else(|| PathBuf::from(".").canonicalize().unwrap());

        let resolved_included_paths: Vec<String> = config
            .included_paths
            .iter()
            .map(|p| start_path.join(p).to_string_lossy().to_string())
            .collect();

        let resolved_excluded_paths: Vec<String> = config
            .excluded_paths
            .iter()
            .map(|p| start_path.join(p).to_string_lossy().to_string())
            .collect();

        let matched_files =
            self.find_and_filter_files(&start_path, &resolved_included_paths, &resolved_excluded_paths)?;

        if show_patterns {
            println!("included:");
            for path in &resolved_included_paths {
                println!("  {}", path);
            }

            println!("excluded:");
            for path in &resolved_excluded_paths {
                println!("  {}", path);
            }
        }

        if show_paths {
            println!("results:");
            for file in &matched_files {
                println!("  {}", file.display());
            }
        }

        if !show_patterns && !show_paths {
            for (index, file) in matched_files.iter().enumerate() {
                self.print_file_content(file, index > 0)?;
            }
        }

        Ok(matched_files)
    }

    fn find_and_filter_files(
        &self,
        base_path: &Path,
        include_patterns: &[String],
        exclude_patterns: &[String],
    ) -> Result<Vec<PathBuf>> {
        let mut include_builder = GlobSetBuilder::new();
        for pat in include_patterns {
            let pattern_path = Path::new(pat);
            let rel_pattern = if pattern_path.is_absolute() {
                pattern_path
                    .strip_prefix(base_path)
                    .unwrap_or(pattern_path)
                    .to_string_lossy()
                    .to_string()
            } else {
                pat.clone()
            };
            include_builder.add(Glob::new(&rel_pattern)?);
        }
        let include_set = include_builder.build()?;

        let mut exclude_builder = GlobSetBuilder::new();
        for pat in exclude_patterns {
            let pattern_path = Path::new(pat);
            let rel_pattern = if pattern_path.is_absolute() {
                pattern_path
                    .strip_prefix(base_path)
                    .unwrap_or(pattern_path)
                    .to_string_lossy()
                    .to_string()
            } else {
                pat.clone()
            };
            exclude_builder.add(Glob::new(&rel_pattern)?);
        }
        let exclude_set = exclude_builder.build()?;

        let mut results = Vec::new();
        for entry in WalkDir::new(base_path) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel_path = entry.path().strip_prefix(base_path)?;
            if include_set.is_match(rel_path) && !exclude_set.is_match(rel_path) {
                results.push(entry.path().to_path_buf());
            }
        }
        Ok(results)
    }

    fn print_file_content(&self, path: &Path, add_spacing: bool) -> Result<()> {
        if add_spacing {
            println!();
        }
        println!("--- {} ---", path.display());

        let bat_available = ShellCommand::new("bat").output().is_ok();
        let viewer = if bat_available { "bat" } else { "cat" };

        let status = ShellCommand::new(viewer)
            .arg(path)
            .status()
            .map_err(|e| eyre!("Failed to run '{}': {}", viewer, e))?;

        if !status.success() {
            return Err(eyre!("{} command failed with status: {}", viewer, status));
        }

        Ok(())
    }
}

/// Handles the “ptns” subcommand by constructing a Config from the matches,
/// building a temporary Kat instance, and immediately running it.
fn handle_ptns_subcommand(sub_m: &ArgMatches, show_patterns: bool, show_paths: bool) -> Result<()> {
    let ptns_config = Config::from_matches("ptns", "ad-hoc pattern run", sub_m);

    // Build a temporary Kat instance with only this “ptns” config
    let mut one_config_map = HashMap::new();
    one_config_map.insert("ptns".to_string(), ptns_config);
    let ad_hoc_kat = Kat { configs: one_config_map };

    // Determine whether the user passed a “path” override
    let path_override = sub_m.get_one::<String>("path").map(PathBuf::from);
    ad_hoc_kat.run_subcommand("ptns", path_override, show_patterns, show_paths)?;
    std::process::exit(0);
}

fn main() -> Result<()> {
    // Set up logging to ~/.cache/kat/kat.log
    let log_file = dirs::cache_dir()
        .map(|p| {
            let log_dir = p.join("kat");
            if !log_dir.exists() {
                fs::create_dir_all(&log_dir).expect("Failed to create log directory");
            }
            log_dir.join("kat.log")
        })
        .unwrap_or_else(|| PathBuf::from("./kat.log"));

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .target(env_logger::Target::Pipe(Box::new(fs::File::create(log_file)?)))
        .init();

    // Load ~/.config/kat/ for YAML configs
    let config_dir = dirs::config_dir()
        .ok_or_else(|| eyre!("Failed to locate config directory"))?
        .join("kat");

    let kat = Kat::new(config_dir)?;

    let args: Vec<String> = std::env::args().collect();
    info!("Parsing arguments: {:?}", args);
    let matches = Kat::parse(&kat.configs, &args)?;

    // If no subcommand was provided, show help and exit
    if matches.subcommand().is_none() {
        println!("{}", Kat::configs_to_command(&kat.configs).render_help());
        std::process::exit(0);
    }

    let show_patterns = matches.get_flag("show-patterns");
    let show_paths = matches.get_flag("show-paths");

    // Handle the ad-hoc “ptns” subcommand
    if let Some(("ptns", sub_m)) = matches.subcommand() {
        handle_ptns_subcommand(sub_m, show_patterns, show_paths)?;
    }

    // Otherwise, handle a normal YAML-based subcommand
    if let Some((subcommand, sub_matches)) = matches.subcommand() {
        let path_override = sub_matches.get_one::<String>("path").map(PathBuf::from);
        kat.run_subcommand(subcommand, path_override, show_patterns, show_paths)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn process_path_for_test(path: PathBuf) -> String {
        path.strip_prefix(std::env::current_dir().unwrap())
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    fn load_config_from_string(config_str: &str) -> Config {
        serde_yaml::from_str(config_str).expect("Failed to parse YAML configuration")
    }

    fn create_kat_with_config(config_name: &str, config_str: &str) -> Kat {
        let mut configs = HashMap::new();
        let mut config: Config = load_config_from_string(config_str);
        config.name = config_name.to_string();
        configs.insert(config_name.to_string(), config);
        Kat { configs }
    }

    #[test]
    fn test_rust_directory() -> Result<()> {
        let rust_config = r#"
        about: "Concatenates Rust-related files"
        included_paths:
          - "Cargo.toml"
          - "build.rs"
          - "src/**/*.rs"
          - "examples/**/*.rs"
          - "tests/**/*.rs"
          - "benches/**/*.rs"
        excluded_paths:
          - "target/**"
          - "**/target/**"
          - "**/incremental/**"
          - "**/.git/**"
          - "**/.idea/**"
          - "**/.DS_Store"
          - "examples/rust/**"
          - "examples/rust/Cargo.toml"
          - "examples/rust/build.rs"
          - "examples/rust/src/**"
        included_types:
          - "rs"
          - "toml"
        excluded_types:
          - "bin"
          - "exe"
          - "o"
          - "so"
        "#;

        let kat = create_kat_with_config("rust", rust_config);
        let matched_files = kat
            .run_subcommand("rust", Some(PathBuf::from("examples/rust")), false, true)?
            .into_iter()
            .map(process_path_for_test)
            .collect::<HashSet<_>>();

        let expected: HashSet<String> = [
            "examples/rust/src/lib/feature1.rs",
            "examples/rust/src/lib/mod.rs",
            "examples/rust/src/main.rs",
            "examples/rust/src/lib/feature2.rs",
            "examples/rust/build.rs",
            "examples/rust/src/lib/config.rs",
            "examples/rust/src/utils/helper1.rs",
            "examples/rust/src/utils/mod.rs",
            "examples/rust/Cargo.toml",
            "examples/rust/src/utils/helper2.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        assert_eq!(matched_files, expected);
        Ok(())
    }

    #[test]
    fn test_python_directory() -> Result<()> {
        let python_config = r#"
        about: "Concatenates Python-related files"
        included_paths:
          - "**/*.py"
          - "**/*.toml"
          - "**/*.yml"
          - "**/*.yaml"
          - "**/*.txt"
          - "**/*.md"
        excluded_paths:
          - "**/.*"
          - ".git/**"
          - ".github/**"
          - "**/build/**"
          - "**/dist/**"
          - "**/*.egg-info/**"
          - "**/venv/**"
          - "**/__pycache__/**"
        included_types:
          - "py"
          - "toml"
          - "yml"
          - "yaml"
          - "txt"
          - "md"
        excluded_types:
          - "bin"
          - "exe"
          - "o"
          - "so"
          - "pyc"
        "#;

        let kat = create_kat_with_config("python", python_config);
        let matched_files = kat
            .run_subcommand("python", Some(PathBuf::from("examples/python")), false, true)?
            .into_iter()
            .map(process_path_for_test)
            .collect::<HashSet<_>>();

        let expected: HashSet<String> = [
            "examples/python/requirements.txt",
            "examples/python/README.md",
            "examples/python/project/core/__init__.py",
            "examples/python/project/core/module2.py",
            "examples/python/setup.py",
            "examples/python/project/__init__.py",
            "examples/python/scripts/deploy.py",
            "examples/python/main.py",
            "examples/python/pyproject.toml",
            "examples/python/project/data/__init__.py",
            "examples/python/docs/manual.md",
            "examples/python/project/data/loader.py",
            "examples/python/scripts/generate_config.py",
            "examples/python/project/core/utils.py",
            "examples/python/project/core/module1.py",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        assert_eq!(matched_files, expected);
        Ok(())
    }

    #[test]
    fn test_yaml_directory() -> Result<()> {
        let yaml_config = r#"
        about: "Handles YAML and YML files only"
        included_paths:
          - "**/*.yml"
          - "**/*.yaml"
        excluded_paths:
          - "build/"
          - "dist/"
          - "**/.git/"
          - "**/__pycache__/"
          - "**/*.egg-info/"
        included_types:
          - "yml"
          - "yaml"
        excluded_types:
          - "bin"
          - "exe"
          - "o"
          - "so"
          - "pyc"
        "#;

        let kat = create_kat_with_config("yaml", yaml_config);
        let matched_files = kat
            .run_subcommand("yaml", Some(PathBuf::from("examples/yaml")), false, true)?
            .into_iter()
            .map(process_path_for_test)
            .collect::<HashSet<_>>();

        let expected: HashSet<String> = [
            "examples/yaml/setup/deploy.yaml",
            "examples/yaml/data/config_backup.yml",
            "examples/yaml/setup/environment.yml",
            "examples/yaml/archive/old_config.yaml",
            "examples/yaml/workflows/release.yaml",
            "examples/yaml/templates/schema.yml",
            "examples/yaml/templates/base.yaml",
            "examples/yaml/archive/unused.yml",
            "examples/yaml/misc/README.yaml",
            "examples/yaml/workflows/ci.yml",
            "examples/yaml/config.yaml",
            "examples/yaml/setup/deploy_backup.yml",
            "examples/yaml/data/data_schema.yaml",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        assert_eq!(matched_files, expected);
        Ok(())
    }

    #[test]
    fn test_toml_directory() -> Result<()> {
        let toml_config = r#"
        about: "Handles TOML files only"
        included_paths:
          - "**/*.toml"
        excluded_paths:
          - "target/"
          - "dist/"
          - "**/.git/"
          - "**/__pycache__/"
          - "**/*.egg-info/"
        included_types:
          - "toml"
        excluded_types:
          - "bin"
          - "exe"
          - "o"
          - "so"
          - "pyc"
        "#;

        let kat = create_kat_with_config("toml", toml_config);
        let matched_files = kat
            .run_subcommand("toml", Some(PathBuf::from("examples/toml")), false, true)?
            .into_iter()
            .map(process_path_for_test)
            .collect::<HashSet<_>>();

        let expected: HashSet<String> = [
            "examples/toml/config.toml",
            "examples/toml/data/config_backup.toml",
            "examples/toml/data/data_schema.toml",
            "examples/toml/setup/deploy.toml",
            "examples/toml/setup/environment.toml",
            "examples/toml/setup/deploy_backup.toml",
            "examples/toml/templates/base.toml",
            "examples/toml/templates/schema.toml",
            "examples/toml/workflows/release.toml",
            "examples/toml/workflows/ci.toml",
            "examples/toml/misc/README.toml",
            "examples/toml/archive/old_config.toml",
            "examples/toml/archive/unused.toml",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        assert_eq!(matched_files, expected);
        Ok(())
    }
}
