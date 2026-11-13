//! The corpus walk's own tests: the forms the evaluator opens a file
//! with, and the resources it reads but never follows.
//!
//! Kept beside the walk, in the repository's `<module>/tests.rs`
//! split, so the walk itself stays within the file size policy.

use super::{expand_under, modules, resources};

/// The forms the evaluator opens a file with are all of them: the
/// import spellings, the two module clauses and the resource a module
/// reads. A form left out of the walk is a file the run opens and the
/// manifest never names, so a change limited to it classifies the
/// mandatory check unaffected.
#[test]
fn the_forms_are_the_ones_the_evaluator_opens() {
    let text = concat!(
        "amends \"base.pkl\"\n",
        "extends \"other.pkl\"\n",
        "value = read(\"notes.txt\").text\n",
    );
    assert_eq!(
        modules(text, "case.pkl").expect("the forms carry their targets"),
        ["base.pkl", "other.pkl"]
    );
    assert_eq!(
        resources(text, "case.pkl").expect("the read carries its target"),
        ["notes.txt"]
    );
}

/// A form in text opens nothing: a commented-out read, a read inside a
/// string and a name that merely reads like one are no file the
/// evaluator opens.
#[test]
fn only_a_call_opens_a_file() {
    let text = concat!(
        "// read(\"commented.txt\")\n",
        "value = \"read(\\\"text.txt\\\")\"\n",
        "read: String\n",
        "value = config.read(\"member.txt\")\n",
    );
    assert_eq!(
        resources(text, "case.pkl").expect("the text carries no target"),
        Vec::<String>::new()
    );
    assert_eq!(
        modules(text, "case.pkl").expect("the text carries no target"),
        Vec::<String>::new()
    );
}

/// A raw string's interpolation is code: the evaluator opens the file
/// the import inside it names, so the walk must find that import too.
#[test]
fn a_raw_string_interpolation_names_its_module() {
    let text = "value = #\"\\#(import(\"../helper.pkl\").value)\"#\n";
    assert_eq!(
        modules(text, "case.pkl").expect("the interpolation carries its target"),
        ["../helper.pkl"]
    );
}

/// A resource is a file the run opens, so it belongs on the manifest —
/// but it is not a module: its bytes are content, not source, so the
/// walk stops at it and never demands the file its text names.
#[test]
fn a_resource_is_named_but_never_walked() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = std::env::temp_dir().join(format!(
        "ruxel-corpus-imports-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir_all(corpus.join("valid"))?;
    std::fs::write(
        corpus.join("valid/example.pkl"),
        "content = read(\"notes.txt\").text\n",
    )?;
    // The resource's text carries an import: a resource is content, so
    // the walk must not read it as a module.
    std::fs::write(
        corpus.join("valid/notes.txt"),
        "import(\"../unused.pkl\")\n",
    )?;
    let mut files = vec!["tests/workflow/valid/example.pkl".to_owned()];
    expand_under(&corpus, &mut files)?;
    assert!(
        files.contains(&"tests/workflow/valid/notes.txt".to_owned()),
        "the resource the corpus reads belongs on the manifest; the list was {files:?}"
    );
    assert!(
        !files.contains(&"tests/workflow/unused.pkl".to_owned()),
        "a resource is content, not source: the walk must not follow it; the list was {files:?}"
    );
    drop(std::fs::remove_dir_all(&corpus));
    Ok(())
}

/// A resource need not be text: Pkl's byte-resource API opens binary
/// files, so the walk must never read one.
#[test]
fn a_resource_that_is_not_text_is_still_a_resource() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = std::env::temp_dir().join(format!(
        "ruxel-corpus-imports-bytes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir_all(corpus.join("valid"))?;
    std::fs::write(
        corpus.join("valid/example.pkl"),
        "content = read(\"blob.bin\").bytes\n",
    )?;
    std::fs::write(corpus.join("valid/blob.bin"), [0xff, 0xfe, 0x00, 0x01])?;
    let mut files = vec!["tests/workflow/valid/example.pkl".to_owned()];
    expand_under(&corpus, &mut files)?;
    assert!(
        files.contains(&"tests/workflow/valid/blob.bin".to_owned()),
        "the resource the corpus reads belongs on the manifest; the list was {files:?}"
    );
    drop(std::fs::remove_dir_all(&corpus));
    Ok(())
}
