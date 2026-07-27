#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Language {
    C,
    Cpp,
    CSharp,
    Css,
    Dart,
    Dockerfile,
    Elixir,
    Go,
    Haskell,
    Html,
    Java,
    JavaScript,
    Json,
    Julia,
    Kotlin,
    Lua,
    Makefile,
    Markdown,
    Nix,
    ObjectiveC,
    Other,
    Perl,
    Php,
    Python,
    R,
    Ruby,
    Rust,
    Scala,
    Shell,
    Sql,
    Swift,
    Toml,
    TypeScript,
    Vue,
    Xml,
    Yaml,
}

impl Language {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cpp => "C++",
            Self::CSharp => "C#",
            Self::Css => "CSS",
            Self::Dart => "Dart",
            Self::Dockerfile => "Dockerfile",
            Self::Elixir => "Elixir",
            Self::Go => "Go",
            Self::Haskell => "Haskell",
            Self::Html => "HTML",
            Self::Java => "Java",
            Self::JavaScript => "JavaScript",
            Self::Json => "JSON",
            Self::Julia => "Julia",
            Self::Kotlin => "Kotlin",
            Self::Lua => "Lua",
            Self::Makefile => "Makefile",
            Self::Markdown => "Markdown",
            Self::Nix => "Nix",
            Self::ObjectiveC => "Objective-C",
            Self::Other => "Other",
            Self::Perl => "Perl",
            Self::Php => "PHP",
            Self::Python => "Python",
            Self::R => "R",
            Self::Ruby => "Ruby",
            Self::Rust => "Rust",
            Self::Scala => "Scala",
            Self::Shell => "Shell",
            Self::Sql => "SQL",
            Self::Swift => "Swift",
            Self::Toml => "TOML",
            Self::TypeScript => "TypeScript",
            Self::Vue => "Vue",
            Self::Xml => "XML",
            Self::Yaml => "YAML",
        }
    }
}

pub(crate) fn classify_path(path: &[u8]) -> Language {
    let filename = path.rsplit(|byte| *byte == b'/').next().unwrap_or(path);
    let lowercase: Vec<u8> = filename.iter().map(u8::to_ascii_lowercase).collect();

    match lowercase.as_slice() {
        b"dockerfile" | b"containerfile" => return Language::Dockerfile,
        b"makefile" | b"gnumakefile" => return Language::Makefile,
        _ => {}
    }

    let Some(dot) = lowercase.iter().rposition(|byte| *byte == b'.') else {
        return Language::Other;
    };
    let extension = &lowercase[dot + 1..];
    match extension {
        b"c" | b"h" => Language::C,
        b"cc" | b"cpp" | b"cxx" | b"hh" | b"hpp" | b"hxx" => Language::Cpp,
        b"cs" => Language::CSharp,
        b"css" | b"scss" | b"sass" | b"less" => Language::Css,
        b"dart" => Language::Dart,
        b"ex" | b"exs" => Language::Elixir,
        b"go" => Language::Go,
        b"hs" | b"lhs" => Language::Haskell,
        b"htm" | b"html" => Language::Html,
        b"java" => Language::Java,
        b"js" | b"jsx" | b"mjs" | b"cjs" => Language::JavaScript,
        b"json" => Language::Json,
        b"jl" => Language::Julia,
        b"kt" | b"kts" => Language::Kotlin,
        b"lua" => Language::Lua,
        b"md" | b"markdown" => Language::Markdown,
        b"nix" => Language::Nix,
        b"m" | b"mm" => Language::ObjectiveC,
        b"pl" | b"pm" => Language::Perl,
        b"php" => Language::Php,
        b"py" | b"pyw" => Language::Python,
        b"r" => Language::R,
        b"rb" => Language::Ruby,
        b"rs" => Language::Rust,
        b"scala" | b"sc" => Language::Scala,
        b"sh" | b"bash" | b"zsh" | b"fish" => Language::Shell,
        b"sql" => Language::Sql,
        b"swift" => Language::Swift,
        b"toml" => Language::Toml,
        b"ts" | b"tsx" | b"mts" | b"cts" => Language::TypeScript,
        b"vue" => Language::Vue,
        b"xml" | b"svg" => Language::Xml,
        b"yaml" | b"yml" => Language::Yaml,
        _ => Language::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_extensions_case_insensitively() {
        assert_eq!(classify_path(b"src/main.rs").name(), "Rust");
        assert_eq!(classify_path(b"web/App.TSX").name(), "TypeScript");
        assert_eq!(classify_path(b"include/value.hpp").name(), "C++");
        assert_eq!(classify_path(b"script.py").name(), "Python");
        assert_eq!(classify_path(b"README.md").name(), "Markdown");
    }

    #[test]
    fn classifies_special_filenames_and_unknown_text_as_other() {
        assert_eq!(classify_path(b"containers/Dockerfile").name(), "Dockerfile");
        assert_eq!(classify_path(b"Makefile").name(), "Makefile");
        assert_eq!(classify_path(b"LICENSE").name(), "Other");
        assert_eq!(classify_path(b"notes.unknown").name(), "Other");
    }
}
