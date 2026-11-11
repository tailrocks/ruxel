//! The pklr defect that shapes `Debian.pkl`: a module the module path
//! serves loses the parent class of its own classes when one of its
//! property types names a class of another module.
//!
//! The repro is two files and a root module:
//!
//! - `core.pkl` (in the module path) holds an `open class Base` and a
//!   `class SecretText`.
//! - `lib.pkl` (in the module path) imports `core.pkl` and holds
//!   `class Holder extends core.Base` with the property
//!   `value: String|core.SecretText`.
//! - the root module imports both and asks `step is core.Base`.
//!
//! pklr answers `false`: `Holder` no longer extends `Base` — it loses the
//! members it inherits, and every class of `lib.pkl` loses its parent the
//! same way. Official Pkl 0.33.0 evaluates the same three files and
//! answers `true`.
//!
//! This is why `Debian.pkl` cannot spell its debconf answer union
//! `Static|core.SecretText`: the step classes of that module would stop
//! being `core.Step`s, and the lowering would refuse every workflow that
//! uses them. The union names `Debian.SecretAnswer`, a class of the module
//! itself, and the secret reaches it through `Debian.secretAnswer`, whose
//! *parameter* names the secret class — a function parameter does not
//! trigger the defect.
//!
//! The first test pins the defect: when pklr fixes it, that test fails and
//! the workaround in `Debian.pkl` can go. The second pins the workaround.

use std::path::{Path, PathBuf};

const CORE: &str = "module core

open class Base {
  hidden fixed kind = \"base\"
}

class SecretText {
  hidden parts: String
}
";

const LIB_WITH_THE_DEFECT: &str = "module lib

import \"modulepath:/core.pkl\" as core

class Holder extends core.Base {
  hidden fixed kind = \"holder\"
  value: String|core.SecretText = \"x\"
}
";

const LIB_WITH_THE_WORKAROUND: &str = "module lib

import \"modulepath:/core.pkl\" as core

class SecretAnswer {
  reference: String
}

class Holder extends core.Base {
  hidden fixed kind = \"holder\"
  value: String|SecretAnswer = \"x\"
}
";

const MAIN: &str = "module main

import \"modulepath:/core.pkl\" as core
import \"modulepath:/lib.pkl\" as lib

step = new lib.Holder {}
isBase: Boolean = step is core.Base
";

/// The files of the repro, written into a directory of their own.
fn repro(directory: &Path, lib: &str) {
    std::fs::create_dir_all(directory.join("mp")).expect("create the module path");
    std::fs::write(directory.join("mp/core.pkl"), CORE).expect("write core.pkl");
    std::fs::write(directory.join("mp/lib.pkl"), lib).expect("write lib.pkl");
    std::fs::write(directory.join("main.pkl"), MAIN).expect("write main.pkl");
}

fn is_base(directory: &Path) -> bool {
    let options = ruxel_config::EvalOptions {
        module_path: vec![directory.join("mp")],
        ..ruxel_config::EvalOptions::default()
    };
    let value = ruxel_config::eval(&directory.join("main.pkl"), &options).expect("the repro evaluates");
    value["isBase"]
        .as_bool()
        .expect("the repro answers isBase")
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "ruxel-pklr-parent-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

#[test]
fn a_property_type_that_names_a_class_of_another_module_breaks_the_parents() {
    let directory = scratch("defect");
    repro(&directory, LIB_WITH_THE_DEFECT);
    // pklr loses the parent class. Official Pkl 0.33.0 answers `true` here.
    assert!(!is_base(&directory), "the defect is gone: pklr fixed it");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_union_of_local_classes_keeps_the_parents() {
    let directory = scratch("workaround");
    repro(&directory, LIB_WITH_THE_WORKAROUND);
    assert!(is_base(&directory), "the parent class is gone");
    let _ = std::fs::remove_dir_all(&directory);
}
