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
                Arg::new("debug")
                    .short('d')
                    .long("debug")
                    .help("print only the paths, not the contents")
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

    pub fn run_subcommand(&self, subcommand: &str, path_override: Option<PathBuf>, debug: bool) -> Result<()> {
        let config = self
            .configs
            .get(subcommand)
            .ok_or_else(|| eyre!("Config for '{}' not found", subcommand))?;

        let start_path = path_override
            .map(fs::canonicalize)
            .transpose()?
            .unwrap_or_else(|| PathBuf::from(".").canonicalize().unwrap());

        let matched_files = self.find_and_filter_files(&start_path, &config.included_paths, &config.excluded_paths)?;

        if debug {
            for file in &matched_files {
                println!("{}", file.display());
            }
            return Ok(());
        }

        for file in matched_files {
            self.print_file_content(&file)?;
        }

        Ok(())
    }

    fn find_and_filter_files(&self, base_path: &Path, include_patterns: &[String], exclude_patterns: &[String]) -> Result<Vec<PathBuf>> {
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
    let debug = matches.get_flag("debug");

    if let Some((subcommand, sub_matches)) = matches.subcommand() {
        let path_override = sub_matches.get_one::<String>("path").map(PathBuf::from);
        kat.run_subcommand(subcommand, path_override, debug)?;
    } else {
        println!("No subcommand provided.");
    }

    Ok(())
}
