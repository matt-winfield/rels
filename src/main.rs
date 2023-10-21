use colored::Colorize;
use regex::Regex;
use std::{collections::HashMap, env, time::SystemTime};

use clap::Parser;
use git2::{Commit, Repository, RepositoryOpenFlags, Tag};

// TODO:
// - Allow option to link to commit in GitHub/GitLab/DevOps/etc
// - Allow option to show commit SHA
// - Option to find via release branch instead of tag

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
        short = 't',
        long,
        default_value = "1y",
        help = "The maximum age ([t]ime) of tags to show, in the format 1y 2mon 3w 4d 5h 6m 7s"
    )]
    age: String,

    #[arg(
        short = 'u',
        long,
        help = "The base URL for JIRA tickets, e.g. `https://jira.example.com/browse/`. If not specified, JIRA ticket numbers will not be linked. If {ticket} is included in the URL, it will be replaced with the ticket number, otherwise it will be appended to end of the URL."
    )]
    jira_url: Option<String>,

    #[arg(
        short = 'r',
        long,
        default_value = "[A-Z]+-[0-9]+",
        help = "The regex to use to match JIRA ticket numbers"
    )]
    jira_regex: String,

    #[arg(
        short,
        long,
        help = "Show all commits, not just those matching the JIRA regex."
    )]
    all: bool,

    #[arg(short, long, help = "Filter by tag name or commit message")]
    filter: Option<String>,
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

fn get_tags<'a>(repo: &'a Repository) -> Vec<Tag<'a>> {
    let mut tags = Vec::new();
    let _ = repo.tag_foreach(|tag_id, _| {
        let tag = repo.find_tag(tag_id).unwrap();
        tags.push(tag);
        true
    });
    tags
}

struct CommitDepthInfo<'a> {
    commit: Commit<'a>,
    depth: usize,
}

/// Get all the parent commits of a commit, up to a maximum depth.
fn get_parent_commits<'a>(
    repo: &'a Repository,
    commit: &Commit<'a>,
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

enum TagCommitsError {
    NoTags,
    Git(git2::Error),
    Regex(regex::Error),
}

impl From<git2::Error> for TagCommitsError {
    fn from(err: git2::Error) -> Self {
        TagCommitsError::Git(err)
    }
}

impl From<regex::Error> for TagCommitsError {
    fn from(err: regex::Error) -> Self {
        TagCommitsError::Regex(err)
    }
}

fn get_tag_commits<'a>(
    repo: &'a Repository,
    max_age: std::time::Duration,
    args: &'a Args,
) -> Result<
    (
        HashMap<std::string::String, CommitTagInfo<'a>>,
        Vec<std::string::String>,
    ),
    TagCommitsError,
> {
    let mut commit_to_tag: HashMap<String, CommitTagInfo> = HashMap::new();
    let mut tag_names = Vec::new();

    for tag in get_tags(&repo) {
        let tag_name = tag.name().ok_or(TagCommitsError::NoTags)?.to_owned();
        tag_names.push(tag_name.clone());

        let commit = repo.find_commit(tag.target()?.id())?;
        if !commit_is_within_duration(&commit, max_age) {
            continue;
        }

        // Add the commit directly referenced by the tag
        add_if_matches_regex(commit.clone(), &mut commit_to_tag, 0, &tag_name, &args)?;

        let parents = get_parent_commits(&repo, &commit, args.depth);
        for parent in parents {
            let parent_id = parent.commit.id().to_string();
            let parent_depth = parent.depth;

            if let Some(existing) = commit_to_tag.get(&parent_id) {
                if existing.depth < parent_depth {
                    continue;
                }
            }

            add_if_matches_regex(
                parent.commit.clone(),
                &mut commit_to_tag,
                parent_depth,
                &tag_name,
                &args,
            )?;
        }
    }

    tag_names.sort();

    return Ok((commit_to_tag, tag_names));
}

fn add_if_matches_regex<'a>(
    commit: Commit<'a>,
    commit_to_tag: &mut HashMap<String, CommitTagInfo<'a>>,
    depth: usize,
    tag_name: &String,
    args: &Args,
) -> Result<(), TagCommitsError> {
    let regex = Regex::new(args.jira_regex.as_str())?;
    let Some(message) = commit.message() else {
        return Ok(());
    };

    let formatted_tickets = regex
        .find_iter(message)
        .map(|regex_match| {
            let ticket = regex_match.as_str().bold().italic();
            format!("{ticket}")
        })
        .collect::<Vec<String>>()
        .join(", ");

    let formatted_tickets = if formatted_tickets.is_empty() {
        "(no tickets)".dimmed().to_string()
    } else {
        formatted_tickets
    };

    let urls = regex
        .find_iter(message)
        .map(|regex_match| {
            let ticket = regex_match.as_str();

            match &args.jira_url {
                Some(url) => {
                    if url.contains("{ticket}") {
                        url.replace("{ticket}", ticket)
                    } else {
                        format!("{}{}", url, ticket)
                    }
                }
                None => ticket.to_owned(),
            }
        })
        .collect::<Vec<String>>();

    if regex.is_match(message) || args.all {
        commit_to_tag.insert(
            commit.id().to_string(),
            CommitTagInfo {
                commit,
                depth,
                tag_name: tag_name.clone(),
                formatted_tickets,
                formatted_urls: urls,
            },
        );
    }
    Ok(())
}

struct CommitTagInfo<'a> {
    commit: Commit<'a>,
    depth: usize,
    tag_name: String,
    formatted_tickets: String,
    formatted_urls: Vec<String>,
}

fn main() {
    let args = Args::parse();
    let repo = get_repo();

    let max_age = duration_str::parse(&args.age).unwrap_or_default();
    let (commit_to_tag, tag_names) = match get_tag_commits(&repo, max_age, &args) {
        Ok((commit_to_tag, tag_names)) => (commit_to_tag, tag_names),
        Err(err) => {
            match err {
                TagCommitsError::Git(err) => {
                    eprintln!("{}", format!("Git error: {}", err).red());
                }
                TagCommitsError::Regex(err) => {
                    eprintln!("{}", format!("Regex error: {}", err).red());
                }
                TagCommitsError::NoTags => {
                    eprintln!("{}", "No tags found!".red());
                }
            }
            std::process::exit(1);
        }
    };

    let tag_to_commits = commit_to_tag
        .iter()
        .fold(HashMap::new(), |mut map, (_, info)| {
            map.entry(&info.tag_name)
                .or_insert_with(Vec::new)
                .push(info);
            map
        });

    for tag_name in tag_names {
        let empty = Vec::new();
        let commits = tag_to_commits.get(&tag_name).unwrap_or(&empty);
        let tag_matches_filter = if let Some(filter) = args.filter.clone() {
            tag_name.contains(&filter)
        } else {
            true
        };

        let filtered_commits = commits
            .iter()
            .filter(|commit| {
                if let Some(filter) = args.filter.clone() {
                    tag_matches_filter || commit.formatted_tickets.contains(&filter)
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();

        if args.filter.is_some() && !tag_matches_filter && filtered_commits.is_empty() {
            continue;
        }

        match filtered_commits.is_empty() {
            true => {
                println!("{}", format!("{} (no entries)", tag_name).dimmed())
            }
            false => {
                println!("{}", tag_name.green().bold())
            }
        }

        for commit in filtered_commits {
            if args.jira_url.is_some() {
                println!(
                    "  {: <10} | {}",
                    commit.formatted_tickets.clone(),
                    commit.formatted_urls.join(", ")
                );
            } else {
                println!("  {}", commit.formatted_tickets.clone());
            }
        }
    }
}
