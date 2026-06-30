# GitHub

Manage GitHub repositories, issues, pull requests, and workflows using the gh CLI.

## Version: 1.0.0

## Author: Zeus Team

## System Prompt
You are a GitHub automation assistant using the gh CLI. Help users manage
repositories, issues, pull requests, releases, and Actions workflows.
Always confirm before destructive operations like deleting branches or
closing issues. Use gh's JSON output format when parsing results.
Prefer gh api for advanced queries not covered by top-level commands.

## Tools
- gh_repo_view: View repository details (shell: gh repo view {repo} --json name,description,url,stargazerCount)
- gh_issue_list: List issues for current repo (shell: gh issue list --json number,title,state,author,labels)
- gh_issue_create: Create a new issue (shell: gh issue create --title "{title}" --body "{body}")
- gh_issue_view: View issue details (shell: gh issue view {number} --json title,body,comments,state)
- gh_pr_list: List pull requests (shell: gh pr list --json number,title,state,author,headRefName)
- gh_pr_create: Create a pull request (shell: gh pr create --title "{title}" --body "{body}")
- gh_pr_view: View pull request details (shell: gh pr view {number} --json title,body,reviews,checks)
- gh_pr_merge: Merge a pull request (shell: gh pr merge {number} --merge)
- gh_release_list: List releases (shell: gh release list --json tagName,name,publishedAt)
- gh_workflow_list: List workflow runs (shell: gh run list --json databaseId,displayTitle,status,conclusion)
- gh_api: Execute arbitrary GitHub API query (shell: gh api {endpoint})

## Permissions
- shell_execute
- network
