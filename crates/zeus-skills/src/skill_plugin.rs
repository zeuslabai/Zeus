//! Wraps an existing [`Skill`] as a [`Plugin`] implementation.

use crate::plugin::Plugin;
use crate::{Skill, ToolImplementation, execute_script, execute_shell};
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

/// Adapter that wraps a [`Skill`] (parsed from SKILL.md) as a [`Plugin`].
#[derive(Debug, Clone)]
pub struct SkillPlugin {
    skill: Skill,
}

impl SkillPlugin {
    /// Wrap an existing skill as a plugin.
    pub fn from_skill(skill: Skill) -> Self {
        Self { skill }
    }

    /// Get a reference to the underlying skill.
    pub fn skill(&self) -> &Skill {
        &self.skill
    }
}

#[async_trait]
impl Plugin for SkillPlugin {
    fn name(&self) -> &str {
        &self.skill.name
    }

    fn version(&self) -> &str {
        &self.skill.version
    }

    fn description(&self) -> &str {
        &self.skill.description
    }

    fn tools(&self) -> Vec<ToolSchema> {
        self.skill
            .tools
            .iter()
            .map(|t| ToolSchema {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            })
            .collect()
    }

    async fn execute_tool(&self, name: &str, args: Value) -> Result<String> {
        let tool = self
            .skill
            .tools
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| Error::Skill(format!("Tool not found: {}", name)))?;

        match &tool.implementation {
            ToolImplementation::Shell { command } => execute_shell(command, &args).await,
            ToolImplementation::Script {
                interpreter,
                script,
            } => execute_script(interpreter, script, &args).await,
            ToolImplementation::Native => {
                Err(Error::Skill("Native tools not yet supported".to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SkillTool;
    use std::path::PathBuf;

    fn make_test_skill() -> Skill {
        Skill {
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            version: "1.2.3".to_string(),
            author: Some("Tester".to_string()),
            system_prompt: "You are helpful.".to_string(),
            tools: vec![SkillTool {
                name: "echo_tool".to_string(),
                description: "Echoes input".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"}
                    }
                }),
                implementation: ToolImplementation::Shell {
                    command: "echo hello".to_string(),
                },
            }],
            permissions: vec!["network".to_string()],
            path: PathBuf::from("."),
            raw_content: String::new(),
            invocation: crate::SkillInvocationPolicy::default(),
            command_dispatch: None,
            metadata: None,
            frontmatter: std::collections::HashMap::new(),
            read_when: vec![],
        }
    }

    #[test]
    fn test_skill_plugin_metadata() {
        let skill = make_test_skill();
        let plugin = SkillPlugin::from_skill(skill);

        assert_eq!(plugin.name(), "test-skill");
        assert_eq!(plugin.version(), "1.2.3");
        assert_eq!(plugin.description(), "A test skill");
    }

    #[test]
    fn test_skill_plugin_tools() {
        let skill = make_test_skill();
        let plugin = SkillPlugin::from_skill(skill);
        let tools = plugin.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo_tool");
        assert_eq!(tools[0].description, "Echoes input");
    }

    #[tokio::test]
    async fn test_skill_plugin_execute_shell_tool() {
        let skill = make_test_skill();
        let plugin = SkillPlugin::from_skill(skill);

        let result = plugin
            .execute_tool("echo_tool", serde_json::json!({}))
            .await
            .expect("async operation should succeed");
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn test_skill_plugin_execute_unknown_tool() {
        let skill = make_test_skill();
        let plugin = SkillPlugin::from_skill(skill);

        let result = plugin
            .execute_tool("nonexistent", serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_skill_plugin_inner_skill() {
        let skill = make_test_skill();
        let plugin = SkillPlugin::from_skill(skill);

        assert_eq!(plugin.skill().author, Some("Tester".to_string()));
        assert_eq!(plugin.skill().permissions, vec!["network".to_string()]);
    }
}
