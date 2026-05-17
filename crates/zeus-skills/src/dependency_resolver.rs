//! Skill Dependency Resolver — automatic installation of skill dependencies.
//!
//! When installing a skill, reads its `## Dependencies` section from SKILL.md
//! and ensures all required skills are present with compatible versions.
//!
//! Features:
//! - Topological sort for correct installation order
//! - Cycle detection to prevent infinite loops
//! - Version constraint validation against installed versions
//! - Dry-run mode to preview what would be installed
//! - Integration with SkillInstaller and SkillVersionRegistry

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::skill_versioning::{
    SkillDependency, SkillVersion, SkillVersionRegistry, parse_dependencies_from_skill_md,
    parse_version_from_skill_md, resolve_dependencies,
};

// ============================================================================
// Dependency graph
// ============================================================================

/// A node in the dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepNode {
    /// Skill name.
    pub name: String,
    /// Version if known (from SKILL.md or registry).
    pub version: Option<SkillVersion>,
    /// Dependencies declared by this skill.
    pub dependencies: Vec<SkillDependency>,
}

/// The full dependency graph for a set of skills.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// Nodes keyed by skill name.
    nodes: HashMap<String, DepNode>,
    /// Adjacency list: skill -> skills it depends on.
    edges: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a skill node to the graph.
    pub fn add_node(&mut self, node: DepNode) {
        let deps: Vec<String> = node.dependencies.iter().map(|d| d.name.clone()).collect();
        self.edges.insert(node.name.clone(), deps);
        self.nodes.insert(node.name.clone(), node);
    }

    /// Get a node by name.
    pub fn get(&self, name: &str) -> Option<&DepNode> {
        self.nodes.get(name)
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Detect cycles in the dependency graph using DFS.
    pub fn find_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for name in self.nodes.keys() {
            if !visited.contains(name.as_str()) {
                self.dfs_cycle(name, &mut visited, &mut rec_stack, &mut path, &mut cycles);
            }
        }
        cycles
    }

    fn dfs_cycle(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(deps) = self.edges.get(node) {
            for dep in deps {
                if !visited.contains(dep.as_str()) {
                    self.dfs_cycle(dep, visited, rec_stack, path, cycles);
                } else if rec_stack.contains(dep.as_str())
                    && let Some(start) = path.iter().position(|p| p == dep)
                {
                    let cycle: Vec<String> = path[start..].to_vec();
                    cycles.push(cycle);
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
    }

    /// Topological sort — returns skills in installation order (dependencies first).
    /// Returns None if there's a cycle.
    pub fn topological_sort(&self) -> Option<Vec<String>> {
        // Collect all node names (including dependency targets not in nodes map)
        let mut all_nodes: HashSet<&str> = HashSet::new();
        for name in self.nodes.keys() {
            all_nodes.insert(name);
        }
        for deps in self.edges.values() {
            for dep in deps {
                all_nodes.insert(dep);
            }
        }

        // BFS-based topological sort (Kahn's algorithm)
        // If A depends on B, then B must come before A.
        // reverse_edges: B -> [A] means "A depends on B"
        let mut reverse_edges: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut in_deg: HashMap<&str, usize> = HashMap::new();

        for &name in &all_nodes {
            in_deg.insert(name, 0);
        }

        for (node, deps) in &self.edges {
            for dep in deps {
                reverse_edges
                    .entry(dep.as_str())
                    .or_default()
                    .push(node.as_str());
                *in_deg.entry(node.as_str()).or_insert(0) += 1;
            }
        }

        let mut queue: VecDeque<&str> = VecDeque::new();
        for (&node, &deg) in &in_deg {
            if deg == 0 {
                queue.push_back(node);
            }
        }

        let mut order = Vec::new();
        while let Some(node) = queue.pop_front() {
            order.push(node.to_string());
            if let Some(dependents) = reverse_edges.get(node) {
                for &dependent in dependents {
                    if let Some(deg) = in_deg.get_mut(dependent) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dependent);
                        }
                    }
                }
            }
        }

        if order.len() == all_nodes.len() {
            Some(order)
        } else {
            None // Cycle detected
        }
    }
}

// ============================================================================
// Dependency resolver
// ============================================================================

/// Plan for resolving dependencies before installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    /// Skills to install, in order (dependencies first).
    pub install_order: Vec<String>,
    /// Skills that are already satisfied.
    pub already_installed: Vec<String>,
    /// Skills with version conflicts.
    pub conflicts: Vec<VersionConflict>,
    /// Whether this plan can be executed without issues.
    pub executable: bool,
    /// Cycle errors if any.
    pub cycles: Vec<Vec<String>>,
}

/// A version conflict between what's installed and what's required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionConflict {
    pub skill_name: String,
    pub installed_version: SkillVersion,
    pub required_by: String,
    pub required_version: String,
}

/// Resolver that builds install plans from dependency graphs.
pub struct DependencyResolver {
    /// Skills directory to scan for SKILL.md files.
    skills_dir: PathBuf,
    /// Version registry of installed skills.
    registry: SkillVersionRegistry,
}

impl DependencyResolver {
    /// Create a new resolver.
    pub fn new(skills_dir: PathBuf, registry: SkillVersionRegistry) -> Self {
        Self {
            skills_dir,
            registry,
        }
    }

    /// Get a reference to the version registry.
    pub fn registry(&self) -> &SkillVersionRegistry {
        &self.registry
    }

    /// Get a mutable reference to the version registry.
    pub fn registry_mut(&mut self) -> &mut SkillVersionRegistry {
        &mut self.registry
    }

    /// Build a dependency graph from a skill's SKILL.md content.
    pub fn build_graph_from_content(&self, skill_name: &str, content: &str) -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        let version = parse_version_from_skill_md(content);
        let deps = parse_dependencies_from_skill_md(content);

        graph.add_node(DepNode {
            name: skill_name.to_string(),
            version,
            dependencies: deps.clone(),
        });

        // Scan installed skills for their dependencies (transitive)
        let mut to_scan: VecDeque<String> = deps.iter().map(|d| d.name.clone()).collect();
        let mut scanned: HashSet<String> = HashSet::new();
        scanned.insert(skill_name.to_string());

        while let Some(dep_name) = to_scan.pop_front() {
            if scanned.contains(&dep_name) {
                continue;
            }
            scanned.insert(dep_name.clone());

            let skill_md_path = self.skills_dir.join(&dep_name).join("SKILL.md");
            if let Ok(dep_content) = std::fs::read_to_string(&skill_md_path) {
                let dep_version = parse_version_from_skill_md(&dep_content);
                let dep_deps = parse_dependencies_from_skill_md(&dep_content);

                for dd in &dep_deps {
                    to_scan.push_back(dd.name.clone());
                }

                graph.add_node(DepNode {
                    name: dep_name,
                    version: dep_version,
                    dependencies: dep_deps,
                });
            } else {
                // Dependency not installed — add as a leaf node
                graph.add_node(DepNode {
                    name: dep_name,
                    version: None,
                    dependencies: vec![],
                });
            }
        }

        graph
    }

    /// Create an installation plan for a skill and its dependencies.
    pub fn plan_install(&self, skill_name: &str, content: &str) -> InstallPlan {
        let graph = self.build_graph_from_content(skill_name, content);

        // Check for cycles
        let cycles = graph.find_cycles();
        if !cycles.is_empty() {
            return InstallPlan {
                install_order: vec![],
                already_installed: vec![],
                conflicts: vec![],
                executable: false,
                cycles,
            };
        }

        // Topological sort
        let topo = match graph.topological_sort() {
            Some(order) => order,
            None => {
                return InstallPlan {
                    install_order: vec![],
                    already_installed: vec![],
                    conflicts: vec![],
                    executable: false,
                    cycles: vec![vec!["cycle detected".to_string()]],
                };
            }
        };

        // Classify each skill
        let mut install_order = Vec::new();
        let mut already_installed = Vec::new();
        let mut conflicts = Vec::new();

        for name in &topo {
            if name == skill_name {
                install_order.push(name.clone());
                continue;
            }

            let node = graph.get(name);
            let is_dep = node.is_some();

            if !is_dep {
                continue;
            }

            // Check version constraint from parent(s)
            let mut satisfied = true;
            for (parent_name, parent_node) in &graph.nodes {
                for dep in &parent_node.dependencies {
                    if dep.name == *name {
                        if let Some(installed) = self.registry.get_version(name) {
                            if !dep.version_req.matches(installed) {
                                conflicts.push(VersionConflict {
                                    skill_name: name.clone(),
                                    installed_version: installed.clone(),
                                    required_by: parent_name.clone(),
                                    required_version: dep.version_req.to_string(),
                                });
                                satisfied = false;
                            }
                        } else {
                            satisfied = false;
                        }
                    }
                }
            }

            if satisfied && self.registry.get(name).is_some() {
                already_installed.push(name.clone());
            } else if !conflicts.iter().any(|c| c.skill_name == *name) {
                install_order.push(name.clone());
            }
        }

        let executable = conflicts.is_empty();

        InstallPlan {
            install_order,
            already_installed,
            conflicts,
            executable,
            cycles: vec![],
        }
    }

    /// Check if all dependencies for a skill are satisfied.
    pub fn check_dependencies(&self, content: &str) -> bool {
        let deps = parse_dependencies_from_skill_md(content);
        let resolution = resolve_dependencies(&deps, &self.registry);
        resolution.all_satisfied
    }
}

// ============================================================================
// Scan skills directory for dependency info
// ============================================================================

/// Scan a skills directory and build a map of skill_name -> (version, dependencies).
pub fn scan_skill_dependencies(
    skills_dir: &Path,
) -> HashMap<String, (Option<SkillVersion>, Vec<SkillDependency>)> {
    let mut result = HashMap::new();

    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return result;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if let Ok(content) = std::fs::read_to_string(&skill_md)
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            let version = parse_version_from_skill_md(&content);
            let deps = parse_dependencies_from_skill_md(&content);
            result.insert(name.to_string(), (version, deps));
        }
    }

    result
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_versioning::VersionReq;

    #[test]
    fn test_graph_add_and_get() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DepNode {
            name: "skill-a".to_string(),
            version: Some(SkillVersion::new(1, 0, 0)),
            dependencies: vec![],
        });
        assert_eq!(graph.len(), 1);
        assert!(graph.get("skill-a").is_some());
        assert!(graph.get("missing").is_none());
    }

    #[test]
    fn test_graph_empty() {
        let graph = DependencyGraph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.len(), 0);
    }

    #[test]
    fn test_topological_sort_linear() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DepNode {
            name: "a".to_string(),
            version: None,
            dependencies: vec![],
        });
        graph.add_node(DepNode {
            name: "b".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("a", VersionReq::Any)],
        });
        graph.add_node(DepNode {
            name: "c".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("b", VersionReq::Any)],
        });

        let order = graph.topological_sort().unwrap();
        let pos_a = order.iter().position(|n| n == "a").unwrap();
        let pos_b = order.iter().position(|n| n == "b").unwrap();
        let pos_c = order.iter().position(|n| n == "c").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_topological_sort_diamond() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DepNode {
            name: "a".to_string(),
            version: None,
            dependencies: vec![],
        });
        graph.add_node(DepNode {
            name: "b".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("a", VersionReq::Any)],
        });
        graph.add_node(DepNode {
            name: "c".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("a", VersionReq::Any)],
        });
        graph.add_node(DepNode {
            name: "d".to_string(),
            version: None,
            dependencies: vec![
                SkillDependency::new("b", VersionReq::Any),
                SkillDependency::new("c", VersionReq::Any),
            ],
        });

        let order = graph.topological_sort().unwrap();
        let pos_a = order.iter().position(|n| n == "a").unwrap();
        let pos_b = order.iter().position(|n| n == "b").unwrap();
        let pos_c = order.iter().position(|n| n == "c").unwrap();
        let pos_d = order.iter().position(|n| n == "d").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_a < pos_c);
        assert!(pos_b < pos_d);
        assert!(pos_c < pos_d);
    }

    #[test]
    fn test_topological_sort_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DepNode {
            name: "a".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("b", VersionReq::Any)],
        });
        graph.add_node(DepNode {
            name: "b".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("a", VersionReq::Any)],
        });

        assert!(graph.topological_sort().is_none());
    }

    #[test]
    fn test_find_cycles_none() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DepNode {
            name: "a".to_string(),
            version: None,
            dependencies: vec![],
        });
        graph.add_node(DepNode {
            name: "b".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("a", VersionReq::Any)],
        });

        let cycles = graph.find_cycles();
        assert!(cycles.is_empty());
    }

    #[test]
    fn test_find_cycles_detected() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DepNode {
            name: "x".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("y", VersionReq::Any)],
        });
        graph.add_node(DepNode {
            name: "y".to_string(),
            version: None,
            dependencies: vec![SkillDependency::new("x", VersionReq::Any)],
        });

        let cycles = graph.find_cycles();
        assert!(!cycles.is_empty());
    }

    #[test]
    fn test_resolver_no_deps() {
        let tmp = std::env::temp_dir().join("zeus_test_resolver_nodeps");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let registry = SkillVersionRegistry::new();
        let resolver = DependencyResolver::new(tmp.clone(), registry);

        let content = "# Simple Skill\n\n## Version: 1.0.0\n\n## Tools\n- hello: Hi\n";
        let plan = resolver.plan_install("simple", content);

        assert!(plan.executable);
        assert!(plan.cycles.is_empty());
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.install_order, vec!["simple"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolver_with_satisfied_deps() {
        let tmp = std::env::temp_dir().join("zeus_test_resolver_satisfied");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("base-tools")).unwrap();
        std::fs::write(
            tmp.join("base-tools/SKILL.md"),
            "# Base Tools\n\n## Version: 1.5.0\n\n## Tools\n- x: y\n",
        )
        .unwrap();

        let mut registry = SkillVersionRegistry::new();
        registry.register("base-tools", SkillVersion::new(1, 5, 0));

        let resolver = DependencyResolver::new(tmp.clone(), registry);

        let content = "# My Skill\n\n## Version: 1.0.0\n\n## Dependencies\n- base-tools ^1.0.0\n\n## Tools\n- a: b\n";
        let plan = resolver.plan_install("my-skill", content);

        assert!(plan.executable);
        assert!(plan.already_installed.contains(&"base-tools".to_string()));
        assert!(plan.install_order.contains(&"my-skill".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolver_with_missing_deps() {
        let tmp = std::env::temp_dir().join("zeus_test_resolver_missing");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let registry = SkillVersionRegistry::new();
        let resolver = DependencyResolver::new(tmp.clone(), registry);

        let content = "# My Skill\n\n## Version: 1.0.0\n\n## Dependencies\n- missing-dep ^1.0.0\n\n## Tools\n- a: b\n";
        let plan = resolver.plan_install("my-skill", content);

        assert!(plan.executable);
        assert!(plan.install_order.contains(&"missing-dep".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolver_version_conflict() {
        let tmp = std::env::temp_dir().join("zeus_test_resolver_conflict");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut registry = SkillVersionRegistry::new();
        registry.register("old-dep", SkillVersion::new(1, 0, 0));

        let resolver = DependencyResolver::new(tmp.clone(), registry);

        let content = "# My Skill\n\n## Dependencies\n- old-dep ^2.0.0\n\n## Tools\n- a: b\n";
        let plan = resolver.plan_install("my-skill", content);

        assert!(!plan.executable);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].skill_name, "old-dep");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolver_check_dependencies_satisfied() {
        let tmp = std::env::temp_dir().join("zeus_test_resolver_check_sat");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut registry = SkillVersionRegistry::new();
        registry.register("utils", SkillVersion::new(2, 0, 0));

        let resolver = DependencyResolver::new(tmp.clone(), registry);
        let content = "# Skill\n\n## Dependencies\n- utils >=1.0.0\n";
        assert!(resolver.check_dependencies(content));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolver_check_dependencies_unsatisfied() {
        let tmp = std::env::temp_dir().join("zeus_test_resolver_check_unsat");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let registry = SkillVersionRegistry::new();
        let resolver = DependencyResolver::new(tmp.clone(), registry);
        let content = "# Skill\n\n## Dependencies\n- missing >=1.0.0\n";
        assert!(!resolver.check_dependencies(content));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_skill_dependencies() {
        let tmp = std::env::temp_dir().join("zeus_test_scan_deps");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("skill-a")).unwrap();
        std::fs::create_dir_all(tmp.join("skill-b")).unwrap();
        std::fs::write(
            tmp.join("skill-a/SKILL.md"),
            "# Skill A\n\n## Version: 1.0.0\n\n## Dependencies\n- skill-b ^1.0\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("skill-b/SKILL.md"),
            "# Skill B\n\n## Version: 1.2.0\n",
        )
        .unwrap();

        let deps = scan_skill_dependencies(&tmp);
        assert_eq!(deps.len(), 2);
        assert!(deps.contains_key("skill-a"));
        assert!(deps.contains_key("skill-b"));

        let (ver_a, deps_a) = &deps["skill-a"];
        assert_eq!(ver_a.as_ref().unwrap(), &SkillVersion::new(1, 0, 0));
        assert_eq!(deps_a.len(), 1);
        assert_eq!(deps_a[0].name, "skill-b");

        let (ver_b, deps_b) = &deps["skill-b"];
        assert_eq!(ver_b.as_ref().unwrap(), &SkillVersion::new(1, 2, 0));
        assert!(deps_b.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = std::env::temp_dir().join("zeus_test_scan_empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let deps = scan_skill_dependencies(&tmp);
        assert!(deps.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_nonexistent_dir() {
        let deps = scan_skill_dependencies(Path::new("/tmp/zeus_nonexistent_dir_12345"));
        assert!(deps.is_empty());
    }

    #[test]
    fn test_plan_cycle_detection() {
        let tmp = std::env::temp_dir().join("zeus_test_plan_cycle");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("dep-a")).unwrap();
        std::fs::write(
            tmp.join("dep-a/SKILL.md"),
            "# Dep A\n\n## Version: 1.0.0\n\n## Dependencies\n- my-skill ^1.0\n",
        )
        .unwrap();

        let registry = SkillVersionRegistry::new();
        let resolver = DependencyResolver::new(tmp.clone(), registry);

        let content = "# My Skill\n\n## Version: 1.0.0\n\n## Dependencies\n- dep-a ^1.0\n\n## Tools\n- x: y\n";
        let plan = resolver.plan_install("my-skill", content);

        assert!(!plan.executable);
        assert!(!plan.cycles.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
