use colored::Colorize;
use std::{collections::HashMap, env, time::SystemTime};

use clap::Parser;
use git2::{Commit, Repository, RepositoryOpenFlags};

#[derive(Parser, Debug)]
struct Args {
    #[arg(
        short,
        long,
        default_value_t = 10,
        help = "Maximum depth to search commits from tags"
    )]
    depth: usize,

    #[arg(
        short,
        long,
        default_value = "1y",
        help = "The maximum age of tags to show, in the format 1y 2mon 3w 4d 5h 6m 7s"
    )]
    age: String,
}

fn get_repo() -> Repository {
    match Repository::open_ext(
        ".",
        RepositoryOpenFlags::empty(),
        &[] as &[&std::ffi::OsStr],
    ) {
        Ok(repo) => repo,
        Err(_) => {
            let path = env::current_dir().unwrap_or(std::path::PathBuf::from("."));
            eprintln!(
                "{}",
                format!(
                    "{} is not a git repository!",
                    path.display().to_string().bold()
                )
                .red()
            );
            std::process::exit(1);
        }
    }
}

struct CommitDepthInfo<'a> {
    commit: Commit<'a>,
    depth: usize,
}

/// Get all the parent commits of a commit, up to a maximum depth.
fn get_parent_commits<'a>(
    repo: &'a Repository,
    commit: Commit<'a>,
    max_depth: usize,
) -> Vec<CommitDepthInfo<'a>> {
    let mut commits = Vec::new();
    let parents = commit.parents();
    let mut commit_ids_to_check = parents.map(|p| p.id()).collect::<Vec<_>>();
    let mut depths: HashMap<_, _> = commit_ids_to_check
        .iter()
        .map(|id| (*id, 1))
        .collect::<HashMap<_, _>>();

    while let Some(parent_id) = commit_ids_to_check.pop() {
        let parent_commit = repo
            .find_commit(parent_id)
            .expect("repo should contain commit");

        let depth = *depths.get(&parent_id).unwrap_or(&1);

        if depth > max_depth {
            continue;
        }

        commit_ids_to_check.extend(parent_commit.parents().map(|p| p.id()));
        parent_commit.parents().for_each(|p| {
            depths.insert(p.id(), depth + 1);
        });

        commits.push(CommitDepthInfo {
            commit: parent_commit,
            depth,
        });
    }
    commits
}

fn commit_is_within_duration(commit: &Commit, max_age: std::time::Duration) -> bool {
    if let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        let commit_time = commit.time().seconds();
        let now_seconds = now.as_secs();

        let diff_seconds = now_seconds - commit_time as u64;
        return diff_seconds < max_age.as_secs();
    }
    true
}

struct CommitTagInfo<'a> {
    commit: Commit<'a>,
    depth: usize,
    tag_name: String,
}

fn main() {
    let args = Args::parse();
    let repo = get_repo();

    let max_age = duration_str::parse(&args.age).unwrap_or_default();

    let mut commit_to_tag: HashMap<String, CommitTagInfo> = HashMap::new();
    let mut tag_names = Vec::new();

    let _ = repo.tag_foreach(|tag_id, _| {
        let tag = repo.find_tag(tag_id).unwrap();
        let tag_name = tag.name().unwrap_or_default().to_owned();
        tag_names.push(tag_name.clone());

        if let Ok(target) = tag.target() {
            if let Ok(commit) = repo.find_commit(target.id()) {
                if !commit_is_within_duration(&commit, max_age) {
                    return true;
                }

                // Add the commit directly referenced by the tag
                commit_to_tag.insert(
                    commit.id().to_string(),
                    CommitTagInfo {
                        commit: commit.clone(),
                        depth: 0,
                        tag_name: tag_name.clone(),
                    },
                );

                let parents = get_parent_commits(&repo, commit, args.depth);
                for parent in parents {
                    let parent_id = parent.commit.id().to_string();
                    let parent_depth = parent.depth;

                    if let Some(existing) = commit_to_tag.get(&parent_id) {
                        if existing.depth < parent_depth {
                            continue;
                        }
                    }

                    commit_to_tag.insert(
                        parent_id,
                        CommitTagInfo {
                            commit: parent.commit,
                            depth: parent_depth,
                            tag_name: tag_name.clone(),
                        },
                    );
                }
            }
        }
        true
    });

    let tag_to_commits = commit_to_tag
        .iter()
        .fold(HashMap::new(), |mut map, (_, info)| {
            map.entry(&info.tag_name)
                .or_insert_with(Vec::new)
                .push(info);
            map
        });

    tag_names.sort();

    for tag_name in tag_names {
        let empty = Vec::new();
        let commits = tag_to_commits.get(&tag_name).unwrap_or(&empty);

        match commits.is_empty() {
            true => {
                println!("{}", format!("{} (no unique commits)", tag_name).dimmed())
            }
            false => {
                println!("{}", tag_name.green().bold())
            }
        }

        for commit in commits {
            println!("  {}", commit.commit.summary().unwrap_or_default());
        }
    }
}
