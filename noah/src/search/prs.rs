use std::time::Instant;

use color_eyre::Result;
use serde::Serialize;

use crate::search::{
    args,
    branches::BranchPlan,
    github::{
        self, BranchReachability, BranchReachabilityRequest,
        BranchReachabilityStatus, GitHubClient, GithubConfig,
        PullRequest, PullRequestState, parse_direct_pr_number,
    },
    render,
    terminal::SearchProgress,
};

const DEFAULT_DAYS: u32 = 15;

pub fn run(json: bool, args: &args::PrsArgs, github_config: &GithubConfig) -> Result<()> {
    let query = args.query.join(" ");
    let direct_pr_number = parse_direct_pr_number(&query);
    let days = args.days.value.unwrap_or(DEFAULT_DAYS);
    let token = github::auth::token(github_config)?;
    let client = GitHubClient::new(token, github_config)?;
    let then = Instant::now();

    let progress = SearchProgress::start(
        json,
        direct_pr_number.map_or_else(
            || format!("Searching NixOS/nixpkgs PRs for {query}..."),
            |number| format!("Fetching NixOS/nixpkgs PR #{number}..."),
        ),
    );

    let prs = if let Some(number) = direct_pr_number {
        client.pull_request(number)?.into_iter().collect::<Vec<_>>()
    } else {
        client.search_prs(&query, days)?
    };

    if prs.iter().any(|pr| {
        matches!(pr.state, PullRequestState::Merged)
            && pr.merge_commit_sha.is_some()
    }) {
        progress.set_message("Checking branch reachability...");
    }

    let plans = prs
        .iter()
        .map(|pr| BranchPlan::from_base_branch(&pr.base_branch))
        .collect::<Vec<_>>();
    let reachability_by_pr =
        probe_reachability_by_pr(&client, &prs, &plans);

    progress.finish();

    if json {
        return print_json(
            query,
            direct_pr_number,
            direct_pr_number.is_none().then_some(days),
            then.elapsed().as_millis(),
            &prs,
            &reachability_by_pr,
        );
    }

    if let Some(number) = direct_pr_number {
        if args.days.value.is_some() {
            eprintln!("Ignoring --days for direct PR lookup.");
        }
        if prs.is_empty() {
            println!("No NixOS/nixpkgs PR found for #{number}.");
            return Ok(());
        }
    } else if prs.is_empty() {
        println!(
            "No PRs found for '{query}' updated in the last {days} days."
        );
        return Ok(());
    }

    render::prs::print(&prs, &plans, &reachability_by_pr);
    Ok(())
}

#[expect(clippy::expect_used, reason = "pr_index is bounded by prs.len()")]
fn probe_reachability_by_pr(
    client: &GitHubClient,
    prs: &[PullRequest],
    plans: &[BranchPlan],
) -> Vec<Vec<BranchReachability>> {
    let mut pr_indexes = Vec::new();
    let mut requests = Vec::new();

    for (pr_index, (pr, plan)) in prs.iter().zip(plans).enumerate() {
        if let Some(sha) = pr.merge_commit_sha.as_deref()
            && matches!(pr.state, PullRequestState::Merged)
        {
            for branch in plan.probe_targets() {
                pr_indexes.push(pr_index);
                requests.push(BranchReachabilityRequest {
                    branch: branch.clone(),
                    commit_sha: sha.to_string(),
                });
            }
        }
    }

    let mut reachability_by_pr = vec![Vec::new(); prs.len()];
    for (pr_index, reachability) in pr_indexes
        .into_iter()
        .zip(client.probe_branch_reachability(&requests))
    {
        // `pr_index` was pushed while enumerating `prs`, so it is always in
        // bounds for a vector sized `prs.len()`.
        reachability_by_pr
            .get_mut(pr_index)
            .expect("pr_index is bounded by prs.len()")
            .push(reachability);
    }

    reachability_by_pr
}

#[derive(Debug, Serialize)]
struct PrSearchJsonOutput<'a> {
    query: String,
    days: Option<u32>,
    direct_pr_number: Option<u64>,
    elapsed_ms: u128,
    results: Vec<PrJsonResult<'a>>,
}

#[derive(Debug, Serialize)]
struct PrJsonResult<'a> {
    number: u64,
    title: &'a str,
    url: &'a str,
    state: &'static str,
    base_branch: &'a str,
    merge_commit_sha: Option<&'a str>,
    reachability: Vec<BranchReachabilityJson<'a>>,
}

#[derive(Debug, Serialize)]
struct BranchReachabilityJson<'a> {
    branch: &'a str,
    status: &'static str,
    reason: Option<&'a str>,
}

fn print_json(
    query: String,
    direct_pr_number: Option<u64>,
    days: Option<u32>,
    elapsed_ms: u128,
    prs: &[PullRequest],
    reachability_by_pr: &[Vec<BranchReachability>],
) -> Result<()> {
    let results = prs
        .iter()
        .zip(reachability_by_pr)
        .map(|(pr, reachability)| {
            let reachability = reachability
                .iter()
                .map(|branch| {
                    let reason = match &branch.status {
                        BranchReachabilityStatus::Unknown(reason) => {
                            Some(reason.as_str())
                        }
                        BranchReachabilityStatus::Contains
                        | BranchReachabilityStatus::Missing => None,
                    };

                    BranchReachabilityJson {
                        branch: branch.branch.as_str(),
                        status: branch.status.as_str(),
                        reason,
                    }
                })
                .collect();

            PrJsonResult {
                number: pr.number,
                title: pr.title.as_str(),
                url: pr.url.as_str(),
                state: pr.state.as_str(),
                base_branch: pr.base_branch.as_str(),
                merge_commit_sha: pr.merge_commit_sha.as_deref(),
                reachability,
            }
        })
        .collect();

    let output = PrSearchJsonOutput {
        query,
        days,
        direct_pr_number,
        elapsed_ms,
        results,
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(
        state: PullRequestState,
        merge_commit_sha: Option<&str>,
    ) -> PullRequest {
        PullRequest {
            number: 42,
            title: "package: 1.0 -> 1.1".to_string(),
            url: "https://github.com/NixOS/nixpkgs/pull/42".to_string(),
            state,
            base_branch: "master".to_string(),
            merge_commit_sha: merge_commit_sha.map(str::to_string),
        }
    }

    #[test]
    fn json_uses_null_days_for_direct_lookup() -> Result<()> {
        let prs = [pr(PullRequestState::Open, None)];
        let reachability = vec![Vec::new()];

        let output = PrSearchJsonOutput {
            query: "#42".to_string(),
            days: None,
            direct_pr_number: Some(42),
            elapsed_ms: 1,
            results: prs
                .iter()
                .zip(&reachability)
                .map(|(pr, reachability)| PrJsonResult {
                    number: pr.number,
                    title: pr.title.as_str(),
                    url: pr.url.as_str(),
                    state: pr.state.as_str(),
                    base_branch: pr.base_branch.as_str(),
                    merge_commit_sha: pr.merge_commit_sha.as_deref(),
                    reachability: reachability
                        .iter()
                        .map(|branch: &BranchReachability| {
                            BranchReachabilityJson {
                                branch: branch.branch.as_str(),
                                status: branch.status.as_str(),
                                reason: None,
                            }
                        })
                        .collect(),
                })
                .collect(),
        };

        let json = serde_json::to_value(output)?;

        assert_eq!(json.get("days"), Some(&serde_json::Value::Null));
        assert_eq!(json.get("direct_pr_number"), Some(&serde_json::json!(42)));
        assert_eq!(
            json.get("results")
                .and_then(serde_json::Value::as_array)
                .and_then(|r| r.first())
                .and_then(|c| c.get("state")),
            Some(&serde_json::json!("open"))
        );
        Ok(())
    }

    #[test]
    fn unknown_reachability_json_includes_reason() -> Result<()> {
        let status = BranchReachabilityStatus::Unknown(
            "comparison unavailable".to_string(),
        );
        let json = BranchReachabilityJson {
            branch: "master",
            status: status.as_str(),
            reason: match &status {
                BranchReachabilityStatus::Unknown(reason) => {
                    Some(reason.as_str())
                }
                BranchReachabilityStatus::Contains
                | BranchReachabilityStatus::Missing => None,
            },
        };

        let value = serde_json::to_value(json)?;

        assert_eq!(value.get("status"), Some(&serde_json::json!("unknown")));
        assert_eq!(value.get("reason"), Some(&serde_json::json!("comparison unavailable")));
        Ok(())
    }
}
