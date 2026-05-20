pub struct ParseArguments {
    args: Vec<String>,
}

pub enum Command {
    Help,
    Move { src: String, dest: String, recursive: bool },
    Copy { src: String, dest: String },
}

impl ParseArguments {
    pub fn new(args: Vec<String>) -> Self {
        ParseArguments { args }
    }

    pub fn parse(&self) -> Result<Command, String> {
        if self.args.len() < 2 {
            return Err("Not enough arguments provided".into());
        }

        let command = self.args[1].to_lowercase();
        let mut flags = Vec::new();
        let mut paths = Vec::new();

        for arg in &self.args[2..] {
            if arg.starts_with("--") {
                flags.push(arg.trim_start_matches("--").to_string());
            } else {
                paths.push(arg.clone());
            }
        }

        match command.as_str() {
            "help" => Ok(Command::Help),

            "move" => {
                if paths.len() < 2 {
                    return Err("Move command requires <src> <dest>".into());
                }

                let recursive = flags.contains(&"recursive".to_string());

                Ok(Command::Move {
                    src: paths[0].clone(),
                    dest: paths[1].clone(),
                    recursive,
                })
            }

            "copy" => {
                if paths.len() < 2 {
                    return Err("Copy command requires <src> <dest>".into());
                }

                Ok(Command::Copy {
                    src: paths[0].clone(),
                    dest: paths[1].clone(),
                })
            }

            _ => Err(format!("Unknown command: {}", command)),
        }
    }
}
