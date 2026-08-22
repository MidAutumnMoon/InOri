#![expect(clippy::use_debug, reason = "Example code")]

use std::time::Duration;

use anyhow::Result;
use ino_shell::{Shell, cmd};

fn main() -> Result<()> {
    let sh = Shell::new()?;
    let command = cmd!(sh, "sleep 5").timeout(Duration::from_secs(3));

    // Run the command with a timeout
    match command.run() {
        Ok(()) => println!("Command completed successfully."),
        Err(err) => eprintln!("Command failed: {err}"),
    }

    // Run the command with a timeout and get stdout
    match command.read() {
        Ok(output) => println!("Command output: {output}"),
        Err(err) => eprintln!("Command failed: {err}"),
    }

    // Run the command with a timeout and get stderr
    match command.read_stderr() {
        Ok(output) => println!("Command stderr: {output}"),
        Err(err) => eprintln!("Command failed: {err}"),
    }

    // Run the command with a timeout and get the full output
    match command.output() {
        Ok(output) => {
            println!("Command completed successfully.{output:?}");
        }
        Err(err) => eprintln!("Command failed: {err}"),
    }

    Ok(())
}
