//! Where mmux puts worktrees, what it calls them, and what a fresh one needs.
//!
//! A worktree is **not** a new concept in the session model: it loads as an ordinary
//! [`Project`](crate::app), whose directory happens to be a linked checkout of another
//! project's repository (see [Architecture](../docs/06-architecture.md)). So this
//! module owns only the things that are genuinely *about* worktrees — the on-disk
//! location, the generated branch names, and copying across the gitignored files a
//! checkout needs before it can build. The git plumbing lives in [`crate::git`]; the
//! lifecycle (create / merge / remove / adopt) lives in [`crate::app`].

use crate::config::WorktreeConfig;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Worktrees live under `~/.mmux/worktrees/<repo-hash>/<branch-slug>`, never inside
/// the repository.
///
/// Keeping them out of the tree is the whole point: nothing to add to `.gitignore`,
/// no second copy of the project for editors, watchers and build tools to crawl, and
/// one directory to sweep if you ever want them all gone. The hash is over the
/// repository's canonical path — the same trick [`crate::tmux::session_name`] uses —
/// so a repo always maps to the same folder, and two repos with the same basename
/// never collide. `None` only when `HOME` can't be resolved.
pub fn root_for(repo: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let canon = crate::config::canonical(repo);
    let mut hash = DefaultHasher::new();
    canon.hash(&mut hash);
    Some(
        PathBuf::from(home)
            .join(".mmux")
            .join("worktrees")
            .join(format!("{:016x}", hash.finish())),
    )
}

/// The directory a branch's worktree gets. Branch names can contain `/` (and worse),
/// so the leaf is a flattened, filesystem-safe rendering of the branch rather than a
/// nested path — a worktree directory is an implementation detail, not somewhere you
/// navigate by hand.
pub fn path_for(repo: &Path, branch: &str) -> Option<PathBuf> {
    Some(root_for(repo)?.join(slug(branch)))
}

/// Flatten a branch name into one safe path component: lowercase alphanumerics, `.`,
/// `_` and `-` survive; everything else (`/`, spaces, colons) becomes `-`, with runs
/// collapsed and the ends trimmed. Never empty, so it can always be joined onto a path.
pub fn slug(branch: &str) -> String {
    let mut out = String::with_capacity(branch.len());
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(['-', '.']).to_string();
    if trimmed.is_empty() {
        "worktree".to_string()
    } else {
        trimmed
    }
}

/// Two words that mean nothing together and are slightly funny apart. Worktree
/// branches are throwaway — cut, merged, deleted within the hour — so naming one is
/// pure friction; mmux offers a name and you press ⏎. The pairing is deliberately a
/// lofty adjective against a mundane noun, which is the reliable joke, and both lists
/// are hand-picked rather than generated so the combinations stay printable and land
/// more often than not. `taken` skips names already used as branches.
///
/// A worktree's box carries its HEAD commit subject underneath the name, so
/// `smug-toaster` stops being anonymous the moment any work lands in it.
pub fn generate_name(taken: &[String]) -> String {
    // Seeded from the clock: mmux never wants the *same* suggestion twice in a row,
    // and nothing here needs to be reproducible.
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e3779b97f4a7c15);
    // Random picks first — that's what makes successive suggestions feel unrelated.
    for _ in 0..64 {
        // xorshift — a couple of lines, and plenty for picking two words.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let adjective = ADJECTIVES[(seed % ADJECTIVES.len() as u64) as usize];
        let noun = NOUNS[((seed / 61) % NOUNS.len() as u64) as usize];
        let name = format!("{adjective}-{noun}");
        if !taken.iter().any(|t| t == &name) {
            return name;
        }
    }
    // Random sampling can't prove a free name exists, so fall back to a full sweep
    // (offset by the seed so it doesn't always land on the same corner of the grid).
    // Only reachable when thousands of the combinations are already branches.
    let offset = (seed % (ADJECTIVES.len() * NOUNS.len()) as u64) as usize;
    for i in 0..ADJECTIVES.len() * NOUNS.len() {
        let at = (offset + i) % (ADJECTIVES.len() * NOUNS.len());
        let name = format!(
            "{}-{}",
            ADJECTIVES[at / NOUNS.len()],
            NOUNS[at % NOUNS.len()]
        );
        if !taken.iter().any(|t| t == &name) {
            return name;
        }
    }
    // Every pairing is spoken for. Vanishingly unlikely, but it must still return
    // something a branch can be called.
    format!("worktree-{:x}", seed & 0xffff)
}

const ADJECTIVES: &[&str] = &[
    "amber", "anxious", "bashful", "bold", "brisk", "chipper", "cosmic", "crisp", "dapper",
    "drowsy", "dusty", "eager", "feral", "frosty", "gentle", "glorious", "gloomy", "grumpy",
    "hasty", "humble", "jolly", "keen", "kindly", "lanky", "lofty", "lucid", "moody", "nifty",
    "nimble", "noble", "opaque", "placid", "plucky", "polite", "prickly", "quirky", "restless",
    "rowdy", "rugged", "rustic", "salty", "sleepy", "sly", "smug", "solemn", "stoic", "tender",
    "tidy", "timid", "unruly", "upbeat", "vague", "velvet", "vivid", "wary", "wistful", "woolly",
    "zesty",
];

const NOUNS: &[&str] = &[
    "anvil",
    "badger",
    "beacon",
    "bison",
    "cactus",
    "comet",
    "doorbell",
    "dumpling",
    "ferret",
    "gizmo",
    "gopher",
    "hamster",
    "hedgehog",
    "igloo",
    "iguana",
    "jigsaw",
    "kazoo",
    "kettle",
    "kiwi",
    "ladder",
    "lantern",
    "lemur",
    "mango",
    "marmot",
    "muffin",
    "noodle",
    "onion",
    "ocelot",
    "otter",
    "pancake",
    "parsnip",
    "pigeon",
    "puffin",
    "quilt",
    "quokka",
    "raccoon",
    "radish",
    "rhubarb",
    "sardine",
    "satchel",
    "seagull",
    "spreadsheet",
    "teapot",
    "toaster",
    "tuba",
    "turnip",
    "ukulele",
    "umbrella",
    "vole",
    "vulture",
    "waffle",
    "walnut",
    "walrus",
    "wombat",
    "wrench",
    "yak",
    "zeppelin",
    "zucchini",
];

/// Copy the gitignored things a fresh checkout needs from `parent` into `new`.
///
/// This is the difference between a worktree you can run and one that greets you with
/// a missing `.env`. Git deliberately never carries these across, and remembering to
/// do it by hand is exactly the chore worktrees are supposed to avoid. Files are
/// copied (they're small and each checkout may drift); directories are **symlinked**
/// (a `node_modules` copy would be absurd). Entries that don't exist are skipped
/// silently — the list is a wish, not a manifest. Returns the entries it brought over
/// (project-relative), for the flash.
///
/// A bare name (`.env`) is looked for at **every** depth [`search_dirs`] reaches, not
/// only at the project root: a monorepo keeps one env file per package, and a checkout
/// that got the root one and nothing else is still broken everywhere that matters. An
/// entry that spells out a path (`apps/web/.env`) still means exactly that path, which
/// is the way out when the sweep brings too much.
pub fn prepare(parent: &Path, new: &Path, cfg: Option<&WorktreeConfig>) -> Vec<String> {
    let mut copied = Vec::new();
    // Walked at most once, and only if some entry is a bare name that needs it.
    let mut dirs: Option<Vec<PathBuf>> = None;
    for entry in crate::config::worktree_copy_list(cfg) {
        // Only ever reach *into* the project: a `../` escape here would copy some
        // unrelated part of the filesystem into a throwaway checkout.
        if entry.contains("..") || Path::new(&entry).is_absolute() {
            continue;
        }
        if Path::new(&entry).components().count() > 1 {
            if bring_over(&parent.join(&entry), &new.join(&entry)) {
                copied.push(entry);
            }
            continue;
        }
        for dir in dirs.get_or_insert_with(|| search_dirs(parent)).iter() {
            let rel = dir.join(&entry);
            if bring_over(&parent.join(&rel), &new.join(&rel)) {
                copied.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    copied
}

/// Bring one entry across: a file is copied, a directory symlinked, parents created as
/// needed. `false` when there was nothing to bring (`from` doesn't exist), when the new
/// checkout already has it, or when the copy failed — all of which are non-events here.
fn bring_over(from: &Path, to: &Path) -> bool {
    if !from.exists() || to.exists() {
        return false;
    }
    if let Some(dir) = to.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if from.is_dir() {
        symlink(from, to)
    } else {
        std::fs::copy(from, to).is_ok()
    }
}

/// Every directory a bare-name copy entry may be found in: the project root (the empty
/// relative path) plus each subdirectory the `ignore` walk is willing to enter.
///
/// Honouring `.gitignore` is what keeps `node_modules`, `target` and `dist` out of this
/// without a hand-kept list of them — a project already declares its heavy trees, and
/// the walk only ever asks about *directories*, so the gitignored files we're actually
/// after are never filtered by it. Hidden directories (`.git` first of all) are skipped
/// and symlinks aren't followed, so the walk can't loop or wander out of the project.
fn search_dirs(parent: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(parent)
        .max_depth(Some(MAX_SEARCH_DEPTH))
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_dir()))
        .filter_map(|e| e.path().strip_prefix(parent).ok().map(Path::to_path_buf))
        .collect()
}

/// How deep [`search_dirs`] looks. Deep enough for where monorepos actually put things
/// (`apps/web/.env`, `packages/db/prisma/.env`), shallow enough that cutting a worktree
/// never waits on a crawl of a big tree.
const MAX_SEARCH_DEPTH: usize = 6;

#[cfg(unix)]
fn symlink(from: &Path, to: &Path) -> bool {
    std::os::unix::fs::symlink(from, to).is_ok()
}

#[cfg(not(unix))]
fn symlink(_from: &Path, _to: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_flattens_branch_names_to_one_safe_component() {
        assert_eq!(slug("brave-otter"), "brave-otter");
        // A path-shaped branch must not become a nested path.
        assert_eq!(slug("feat/JIRA-12/thing"), "feat-jira-12-thing");
        assert_eq!(slug("weird  name:here"), "weird-name-here");
        // Leading/trailing junk is trimmed, and nothing ever yields an empty component.
        assert_eq!(slug("//edge//"), "edge");
        assert_eq!(slug("///"), "worktree");
    }

    #[test]
    fn generated_names_are_two_words_and_avoid_taken_ones() {
        let name = generate_name(&[]);
        assert_eq!(
            name.split('-').count(),
            2,
            "{name} should be adjective-noun"
        );
        assert_eq!(slug(&name), name, "a generated name is already path-safe");

        // Every name but one is spoken for: generation must find the survivor.
        let mut taken: Vec<String> = Vec::new();
        for a in ADJECTIVES {
            for n in NOUNS {
                taken.push(format!("{a}-{n}"));
            }
        }
        let survivor = taken.pop().unwrap();
        assert_eq!(generate_name(&taken), survivor);
    }

    #[test]
    fn wordlists_are_clean_and_distinct() {
        for list in [ADJECTIVES, NOUNS] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len(), "duplicate word in list");
            assert!(list
                .iter()
                .all(|w| w.chars().all(|c| c.is_ascii_lowercase())));
        }
    }

    #[test]
    fn root_is_per_repository_and_stable() {
        if std::env::var_os("HOME").is_none() {
            return;
        }
        let a = root_for(Path::new("/tmp/one")).unwrap();
        assert_eq!(a, root_for(Path::new("/tmp/one")).unwrap());
        assert_ne!(a, root_for(Path::new("/tmp/two")).unwrap());
        assert!(a.ends_with(a.file_name().unwrap()));
        assert!(a.to_string_lossy().contains(".mmux/worktrees"));
    }

    /// The coupling `sync_worktree_projects` rests on: a checkout mmux creates always
    /// sits under the root it filters adoption by. Break this and either nothing is
    /// adopted, or someone else's worktrees are.
    #[test]
    fn created_paths_live_under_the_adoption_root() {
        if std::env::var_os("HOME").is_none() {
            return;
        }
        let repo = Path::new("/tmp/some-repo");
        let root = root_for(repo).unwrap();
        for branch in ["brave-otter", "feat/deep/name", "WEIRD Name"] {
            let path = path_for(repo, branch).unwrap();
            assert!(
                path.starts_with(&root),
                "{branch}: {path:?} not under {root:?}"
            );
            // …and one component deep, so the branch can never climb out of it.
            assert_eq!(path.parent(), Some(root.as_path()));
        }
        // A different repository gets a different root, so adoption can't cross repos.
        let other = root_for(Path::new("/tmp/other-repo")).unwrap();
        assert!(!path_for(repo, "x").unwrap().starts_with(other));
    }

    #[test]
    fn prepare_copies_files_links_dirs_and_ignores_escapes() {
        let root = std::env::temp_dir().join(format!("mmux-wt-prep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (parent, new) = (root.join("parent"), root.join("new"));
        std::fs::create_dir_all(parent.join("node_modules")).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(parent.join(".env"), "SECRET=1").unwrap();
        std::fs::write(root.join("outside"), "no").unwrap();

        let cfg = WorktreeConfig {
            copy: Some(vec![
                ".env".into(),
                "node_modules".into(),
                "missing".into(),
                "../outside".into(),
            ]),
            setup: None,
            reap: None,
        };
        let copied = prepare(&parent, &new, Some(&cfg));

        assert_eq!(copied, vec![".env".to_string(), "node_modules".to_string()]);
        assert_eq!(
            std::fs::read_to_string(new.join(".env")).unwrap(),
            "SECRET=1"
        );
        // A directory is linked, not duplicated.
        assert!(new.join("node_modules").is_dir());
        assert!(std::fs::symlink_metadata(new.join("node_modules"))
            .unwrap()
            .file_type()
            .is_symlink());
        // The `..` entry is refused outright rather than reaching outside the project.
        assert!(!new.join("outside").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn prepare_finds_a_bare_name_at_every_depth() {
        let root = std::env::temp_dir().join(format!("mmux-wt-nest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (parent, new) = (root.join("parent"), root.join("new"));
        std::fs::create_dir_all(parent.join("apps/web")).unwrap();
        std::fs::create_dir_all(parent.join("packages/db")).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(parent.join(".env"), "ROOT").unwrap();
        std::fs::write(parent.join("apps/web/.env"), "WEB").unwrap();
        std::fs::write(parent.join("packages/db/.env"), "DB").unwrap();
        // A path-shaped entry means that path and nothing else — so this one must not
        // also pick up the `.env.local` two directories down.
        std::fs::write(parent.join("apps/web/.env.local"), "WEB LOCAL").unwrap();

        let cfg = WorktreeConfig {
            copy: Some(vec![".env".into(), "apps/web/.env.local".into()]),
            setup: None,
            reap: None,
        };
        let mut copied = prepare(&parent, &new, Some(&cfg));
        copied.sort();
        assert_eq!(
            copied,
            vec![
                ".env".to_string(),
                "apps/web/.env".to_string(),
                "apps/web/.env.local".to_string(),
                "packages/db/.env".to_string(),
            ]
        );
        // Each one lands where it came from, with its own contents.
        for (path, want) in [
            (".env", "ROOT"),
            ("apps/web/.env", "WEB"),
            ("packages/db/.env", "DB"),
        ] {
            assert_eq!(std::fs::read_to_string(new.join(path)).unwrap(), want);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
