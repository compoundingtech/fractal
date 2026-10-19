//! Read retained generation definitions; never execute activation or collision scripts.
use super::*;
use std::collections::BTreeSet;

pub(super) fn preflight(layout: &Layout) -> Result<()> {
    let config = layout.directories[0].0.join("config.toml");
    let protected = [config.as_path(), layout.directories[1].0.as_path()];
    let mut unsafe_generations = BTreeSet::new();
    for profile in layout.profiles.as_ref().context("USER is required to locate retained Home Manager generations")? {
        let entries = match fs::read_dir(profile) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("inspect Home Manager profile {}", profile.display())),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(number) = name.to_str().and_then(|name| name.strip_prefix("home-manager-")?.strip_suffix("-link")) else { continue; };
            let Ok(number) = number.parse::<u64>() else { continue; };
            ensure!(entry.file_type()?.is_symlink(), "retained Home Manager generation {number} is not a profile link: {}", entry.path().display());
            let forced = generation_forced_paths(&entry.path(), &layout.home)
                .with_context(|| format!("cannot inspect retained Home Manager generation {number} at {}; cutover refused", entry.path().display()))?;
            // Home Manager uses [[ $targetPath == $forcedPath* ]], not path components.
            if forced.iter().any(|forced| protected.iter().any(|path| path.as_os_str().as_bytes().starts_with(forced.as_bytes()))) {
                unsafe_generations.insert(number);
            }
        }
    }
    ensure!(unsafe_generations.is_empty(),
        "retained Home Manager generations have forced targets that bypass legacy config/state collision checks; prune generation numbers [{}] before host cutover",
        unsafe_generations.iter().map(u64::to_string).collect::<Vec<_>>().join(", "));
    Ok(())
}

fn generation_forced_paths(generation: &Path, home: &Path) -> Result<Vec<String>> {
    // Deployment instrumentation keeps the original adjacent to its wrapper.
    let original = generation.join("activate-uninstrumented");
    let activation = if original.try_exists()? { original } else { generation.join("activate") };
    let source = fs::read_to_string(&activation)?;
    let mut found = source.contains("forcedPaths=(");
    let mut paths = if found { forced_paths(&source, home)? } else { Vec::new() };
    for token in source.split_whitespace() {
        let token = token.trim_matches(['\'', '"']);
        if token.ends_with("-check-link-targets.sh") && Path::new(token).is_absolute() {
            found = true;
            paths.extend(forced_paths(&fs::read_to_string(token)?, home)?);
        }
    }
    ensure!(found, "unrecognized Home Manager collision-check definition in {}", activation.display());
    Ok(paths)
}

// Parse only the literal array grammar emitted by lib.escapeShellArg, with HOME
// expansion. Unknown shell expressions fail closed rather than running old code.
fn forced_paths(source: &str, home: &Path) -> Result<Vec<String>> {
    // Inspect the whole script before returning the initial array: any later
    // append/indexed assignment (or other use) invalidates that initial value.
    let references = source.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| *token == "forcedPaths").count();
    ensure!(references == 2
        && source.lines().filter(|line| line.trim_start().starts_with("forcedPaths=(")).count() == 1
        && source.lines().filter(|line| line.trim() == "for forcedPath in \"${forcedPaths[@]}\"; do").count() == 1,
        "unsupported forcedPaths references: expected one initializer and only the pinned checker's read-only loop");
    let (_, array) = source.split_once("forcedPaths=(").context("missing forcedPaths array")?;
    let home = home.to_str().context("Home Manager HOME is not UTF-8")?;
    let mut chars = array.chars().peekable();
    let mut quote = None;
    let mut word = String::new();
    let mut started = false;
    let mut paths = Vec::new();
    while let Some(character) = chars.next() {
        if quote == Some('\'') {
            if character == '\'' { quote = None; } else { word.push(character); }
            continue;
        }
        match character {
            '\'' if quote.is_none() => { quote = Some('\''); started = true; }
            '"' => { quote = if quote.is_some() { None } else { Some('"') }; started = true; }
            '\\' => {
                let escaped = chars.next().context("unterminated shell escape")?;
                if quote == Some('"') && !matches!(escaped, '$' | '`' | '"' | '\\' | '\n') { word.push('\\'); }
                if escaped != '\n' { word.push(escaped); started = true; }
            }
            '$' => {
                let braced = chars.peek() == Some(&'{');
                if braced { chars.next(); }
                for expected in "HOME".chars() {
                    ensure!(chars.next() == Some(expected), "unsupported shell expansion in forcedPaths");
                }
                if braced {
                    ensure!(chars.next() == Some('}'), "unsupported shell expansion in forcedPaths");
                } else {
                    ensure!(!chars.peek().is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_'), "unsupported shell variable in forcedPaths");
                }
                word.push_str(home);
                started = true;
            }
            ')' if quote.is_none() => {
                if started { paths.push(word); }
                return Ok(paths);
            }
            character if quote.is_none() && character.is_whitespace() => {
                if started { paths.push(std::mem::take(&mut word)); started = false; }
            }
            '`' => bail!("unsupported command substitution in forcedPaths"),
            '(' | ';' | '|' | '&' | '<' | '>' | '*' | '?' | '[' | '{' | '}' | '~' | '#' if quote.is_none() => bail!("unsupported shell expression in forcedPaths"),
            character => { word.push(character); started = true; }
        }
    }
    bail!("unterminated forcedPaths array")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_array_grammar_preserves_literals_and_expands_only_home() -> Result<()> {
        let source = "forcedPaths=(\"$HOME\"/'config/st3/config' \\\n            '${HOME}/literal' \"${HOME}/space path\" 'quote'\\''path' \"back\\\\slash\" )\nfor forcedPath in \"${forcedPaths[@]}\"; do\n:\ndone\n";
        assert_eq!(forced_paths(source, Path::new("/tmp/hm-fixture"))?, vec![
            "/tmp/hm-fixture/config/st3/config", "${HOME}/literal", "/tmp/hm-fixture/space path",
            "quote'path", "back\\slash",
        ]);
        Ok(())
    }

    #[test]
    fn dynamic_or_unknown_arrays_are_refused_without_shell_evaluation() {
        for array in [
            "forcedPaths=($(touch sentinel))", "forcedPaths=($OTHER)",
            "forcedPaths=($HOME_SUFFIX)", "forcedPaths=(~/.config)",
            "forcedPaths=({a,b})", "forcedPaths=('unterminated)",
            "forcedPaths=(", "forcedPaths=() forcedPaths=()",
        ] {
            let source = format!("{array}\nfor forcedPath in \"${{forcedPaths[@]}}\"; do\n:\ndone\n");
            assert!(forced_paths(&source, Path::new("/tmp/hm-fixture")).is_err(), "{array}");
        }
    }

    #[test]
    fn later_array_mutations_are_not_hidden_by_the_initial_literal() {
        let upstream = "forcedPaths=()\nfor forcedPath in \"${forcedPaths[@]}\"; do\n:\ndone\n";
        assert!(forced_paths(upstream, Path::new("/tmp/hm-fixture")).is_ok());
        for mutation in ["forcedPaths+=(\"$HOME\"/config/st3/config)", "forcedPaths[0]=\"$HOME/config/st3/config\""] {
            let source = format!("{upstream}{mutation}\n");
            assert!(forced_paths(&source, Path::new("/tmp/hm-fixture")).is_err(), "{mutation}");
        }
    }
}
