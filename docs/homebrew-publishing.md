# Homebrew Publishing

This note summarizes what Journey needs before publishing through Homebrew.

Sources checked on 2026-06-25:

- Homebrew Formula Cookbook: https://docs.brew.sh/Formula-Cookbook
- Homebrew Acceptable Formulae: https://docs.brew.sh/Acceptable-Formulae
- Homebrew tap guide: https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap

## Recommended Path

Start with a project tap, then consider `homebrew/core` after Journey has a public release history and enough external usage.

Why:

- Anyone can create a tap, and GitHub tap repos are conventionally named `homebrew-<name>`.
- Tap formulae use the same Ruby formula format as core formulae.
- `homebrew/core` has extra requirements for supported platforms, notability, tagged releases, homepage, license, audit, and maintainability.

## Release Prerequisites

Before publishing:

- Add a public repository URL to `Cargo.toml`.
- Add a `homepage` target, usually the GitHub repo.
- Ensure `license = "MIT"` is backed by a root `LICENSE` file.
- Keep `Cargo.lock` committed because Journey is an application.
- Tag a stable release, for example `v0.1.0`.
- Publish a GitHub source tarball for that tag.
- Compute the tarball SHA-256.
- Verify `cargo install --locked --path .` works from a clean checkout.
- Verify `cargo test --locked` passes.

## Formula Skeleton

For a tap formula at `Formula/journey.rb`:

```ruby
class Journey < Formula
  desc "Context persistence for engineering efforts"
  homepage "https://github.com/OWNER/journey"
  url "https://github.com/OWNER/journey/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_RELEASE_TARBALL_SHA256"
  license "MIT"

  depends_on "rust" => :build
  depends_on "git"

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    ENV["JOURNEY_HOME"] = testpath/".journey"
    system bin/"journey", "new", "Homebrew Test", "--description", "formula test"
    assert_match "homebrew-test", shell_output("#{bin}/journey list --non-interactive")
    assert_path_exists testpath/".journey/journeys/homebrew-test/journey.yaml"
  end
end
```

Notes:

- Use `std_cargo_args`; Homebrew defines it as `--locked`, `--root=#{root}`, and `--path=#{path}`.
- `git` is a runtime dependency because linking and worktree actions shell out to `git`.
- The test should exercise real behavior without requiring interactive input. Creating a Journey and listing it is better than only checking `--help`.

## Validation Commands

For a local tap:

```sh
brew tap-new OWNER/homebrew-tap
brew create https://github.com/OWNER/journey/archive/refs/tags/v0.1.0.tar.gz --tap OWNER/homebrew-tap --set-name journey
HOMEBREW_NO_INSTALL_FROM_API=1 brew install --build-from-source --verbose --debug journey
brew test journey
brew audit --new --formula journey
brew audit --strict --online journey
```

For a tap release:

```sh
git -C "$(brew --repository OWNER/tap)" add Formula/journey.rb
git -C "$(brew --repository OWNER/tap)" commit -m "journey 0.1.0"
git -C "$(brew --repository OWNER/tap)" push
```

Users install from the tap with:

```sh
brew tap OWNER/tap
brew install journey
```

## `homebrew/core` Checklist

For a core PR, Journey must:

- build and pass tests on Homebrew-supported macOS and Linux targets;
- have a stable upstream tag;
- be source-built, not binary-only;
- have a homepage;
- use an acceptable open-source license;
- be known and used by people other than the author;
- avoid self-update behavior;
- avoid unversioned or unchecksummed downloads;
- pass `brew audit --new --formula journey`;
- be submitted as one formula per commit with the commit message `journey 0.1.0 (new formula)`.

## Open Questions Before Release

- What is the public GitHub owner/repo URL?
- Should the first tap be `OWNER/homebrew-journey` or a broader `OWNER/homebrew-tap`?
- Should the formula include generated shell completions or man pages in a later release?
