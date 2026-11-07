/// How a ported command ended.
pub enum Outcome {
    /// Finished: the exit code and what to print.
    Done { code: i32, stdout: String, stderr: String },
    /// Let the Ruby qm run this command instead.
    Delegate,
}

pub fn done(stdout: String) -> Outcome {
    Outcome::Done { code: 0, stdout, stderr: String::new() }
}

/// Ruby's `abort(message)`: the message on stderr, exit 1.
pub fn abort(message: &str) -> Outcome {
    Outcome::Done { code: 1, stdout: String::new(), stderr: format!("{message}\n") }
}
