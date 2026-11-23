use vergen_gitcl::{Build, Emitter, Gitcl, Rustc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=VERGEN_GIT_SHA");

    let build = Build::builder().build_date(true).build();
    let rustc = Rustc::builder().semver(true).build();
    let git = Gitcl::builder().sha(true).dirty(true).build();

    let mut emitter = Emitter::default();
    emitter
        .fail_on_error()
        .add_instructions(&build)?
        .add_instructions(&rustc)?;

    // Builds without a git worktree take the SHA from VERGEN_GIT_SHA.
    if emitter.add_instructions(&git).is_err() {
        let sha = std::env::var("VERGEN_GIT_SHA").unwrap_or_else(|_| "unknown".into());
        println!("cargo:rustc-env=VERGEN_GIT_SHA={sha}");
        println!("cargo:rustc-env=VERGEN_GIT_DIRTY=unknown");
    }

    emitter.emit()?;
    Ok(())
}
