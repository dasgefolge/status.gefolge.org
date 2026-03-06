use {
    std::{
        env,
        fs::File,
        io::prelude::*,
        path::Path,
    },
    itertools::Itertools as _,
};

fn main() {
    // only do a full rebuild if the git commit hash changed (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
    println!("cargo::rerun-if-changed=.git");
    let mut f = File::create(Path::new(&env::var_os("OUT_DIR").unwrap()).join("version.rs")).unwrap();
    let commit_hash = if let Ok(commit_hash) = env::var("GIT_COMMIT_HASH") {
        if commit_hash.is_empty() {
            writeln!(&mut f, "pub const GIT_COMMIT_HASH: gix::ObjectId = gix::ObjectId::Sha1([0; _]);").unwrap(); // dummy commit hash for testing Nix build
            return
        }
        commit_hash.parse().unwrap()
    } else {
        let repo = gix::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        repo.head_id().unwrap().detach()
    };
    match commit_hash.kind() {
        gix::hash::Kind::Sha1 => writeln!(&mut f, "pub const GIT_COMMIT_HASH: gix::ObjectId = gix::ObjectId::Sha1([{:#x}]);", commit_hash.as_bytes().iter().format(", ")).unwrap(),
        _ => unreachable!("enum marked as nonexhaustive"),
    }
}
