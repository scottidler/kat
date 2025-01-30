use clap::{Arg, ArgMatches, Command};
use eyre::{eyre, Result};
use glob::glob;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command as ShellCommand,
};

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

impl Kat {
    fn new(config_dir: PathBuf) -> Result<Self> {
        let configs = Kat::load_configs(&config_dir)?;
        Ok(Self { configs })
    }

    fn load_configs(config_dir: &Path) -> Result<Configs> {
        if !config_dir.exists() {
            return Err(eyre!("Config directory not found: {}", config_dir.display()));
        }

        let mut configs = Configs::new();

        for entry in fs::read_dir(config_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(extension) = path.extension() {
                    if extension == "yml" || extension == "yaml" {
                        let config_content = fs::read_to_string(&path)?;
                        let mut config: Config = serde_yaml::from_str(&config_content)?;

                        if let Some(file_name) = path.file_stem() {
                            if let Some(name) = file_name.to_str() {
                                config.name = name.to_string();
                                configs.insert(name.to_string(), config);
                            }
                        }
                    }
                }
            }
        }

        Ok(configs)
    }

    fn config_to_command(config: &Config) -> Command {
        let command = Command::new(&config.name)
            .about(&config.about)
            .arg(
                Arg::new("path")
                    .value_name("PATH")
                    .default_value(".")
                    .help("Path to start from (file or directory)")
                    .required(false),
            )
            .arg(
                Arg::new("included-paths")
                    .long("included-paths")
                    .help("Included paths")
                    .default_values(&config.included_paths),
            )
            .arg(
                Arg::new("excluded-paths")
                    .long("excluded-paths")
                    .help("Excluded paths")
                    .default_values(&config.excluded_paths),
            )
            .arg(
                Arg::new("included-types")
                    .long("included-types")
                    .help("Included types")
                    .default_values(&config.included_types),
            )
            .arg(
                Arg::new("excluded-types")
                    .long("excluded-types")
                    .help("Excluded types")
                    .default_values(&config.excluded_types),
            );
        command
    }

    pub fn configs_to_command(configs: &Configs) -> Command {
        let mut command = Command::new("kat")
            .about("Concatenate files with metadata")
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

        for config in configs.values() {
            let subcommand = Kat::config_to_command(config);
            command = command.subcommand(subcommand);
        }

        command
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
            for file in &matched_files {
                self.print_file_content(file)?;
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
        let mut included_files = HashSet::new();

        for pattern in include_patterns {
            let full_pattern = base_path.join(pattern).to_string_lossy().to_string();
            for entry in glob(&full_pattern)? {
                if let Ok(path) = entry {
                    if path.is_file() && !self.is_excluded(&path, base_path, exclude_patterns)? {
                        included_files.insert(path);
                    }
                }
            }
        }

        Ok(included_files.into_iter().collect())
    }

    fn is_excluded(&self, path: &Path, base_path: &Path, exclude_patterns: &[String]) -> Result<bool> {
        for pattern in exclude_patterns {
            let full_pattern = base_path.join(pattern).to_string_lossy().to_string();
            if let Ok(glob_iter) = glob(&full_pattern) {
                for entry in glob_iter {
                    if let Ok(excluded_path) = entry {
                        if path.starts_with(&excluded_path) {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    fn print_file_content(&self, path: &Path) -> Result<()> {
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

fn main() -> Result<()> {
    env_logger::init();

    let config_dir = dirs::config_dir()
        .ok_or_else(|| eyre!("Failed to locate config directory"))?
        .join("kat");

    let kat = Kat::new(config_dir)?;

    let args: Vec<String> = std::env::args().collect();
    let matches = Kat::parse(&kat.configs, &args)?;

    if matches.subcommand().is_none() {
        println!("{}", Kat::configs_to_command(&kat.configs).render_help());
        std::process::exit(0);
    }

    let show_patterns = matches.get_flag("show-patterns");
    let show_paths = matches.get_flag("show-paths");

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
        let config: Config = load_config_from_string(config_str);
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
          - "examples/rust/*.rs"
          - "examples/rust/**/.rs"
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
