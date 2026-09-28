use std::io::BufReader;

use literium::cli::Cli;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let mut cli = Cli {
        // "." like the Perl CLI: user-facing messages echo the
        // invocation directory as given, not canonicalized.
        cwd: std::path::PathBuf::from("."),
        input: &mut input,
        out: &mut out,
        err: &mut err,
    };
    std::process::exit(cli.run(&argv));
}
