use anyhow::Result;
use std::io::{self, BufRead, Write};

/// Prints `prompt` and returns one trimmed line from stdin (empty at end of input).
pub fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

/// Asks a yes/no question; only `y` or `Y` means yes.
pub fn confirm(prompt: &str) -> Result<bool> {
    Ok(read_line(prompt)?.eq_ignore_ascii_case("y"))
}
