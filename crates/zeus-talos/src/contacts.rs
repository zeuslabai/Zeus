//! Contacts tools (macOS)

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// Get full contact details
pub struct ContactsGetTool;

#[async_trait]
impl TalosTool for ContactsGetTool {
    fn name(&self) -> &'static str {
        "contacts_get"
    }
    fn description(&self) -> &'static str {
        "Get full details for a contact by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Contact name to look up",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let escaped = crate::sanitize_applescript(name);
            let script = format!(
                r#"
                set contactInfo to ""
                tell application "Contacts"
                    set matchingPeople to (every person whose name contains "{}")
                    if (count of matchingPeople) > 0 then
                        set p to item 1 of matchingPeople
                        set contactInfo to "Name: " & (name of p) & linefeed
                        if (count of emails of p) > 0 then
                            repeat with e in emails of p
                                set contactInfo to contactInfo & "Email: " & (value of e) & " (" & (label of e) & ")" & linefeed
                            end repeat
                        end if
                        if (count of phones of p) > 0 then
                            repeat with ph in phones of p
                                set contactInfo to contactInfo & "Phone: " & (value of ph) & " (" & (label of ph) & ")" & linefeed
                            end repeat
                        end if
                        if organization of p is not missing value then
                            set contactInfo to contactInfo & "Organization: " & (organization of p) & linefeed
                        end if
                        if job title of p is not missing value then
                            set contactInfo to contactInfo & "Title: " & (job title of p) & linefeed
                        end if
                    else
                        set contactInfo to "Contact not found"
                    end if
                end tell
                return contactInfo
            "#,
                escaped
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Contacts tools only available on macOS".to_string())
        }
    }
}

/// Search contacts
pub struct ContactsSearchTool;

#[async_trait]
impl TalosTool for ContactsSearchTool {
    fn name(&self) -> &'static str {
        "contacts_search"
    }
    fn description(&self) -> &'static str {
        "Search contacts by name or email"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "query",
            "string",
            "Search query",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing query".to_string()))?;

            let script = format!(
                r#"
                set contactList to ""
                tell application "Contacts"
                    set matchingPeople to (every person whose name contains "{}")
                    repeat with p in matchingPeople
                        set contactList to contactList & (name of p)
                        if (count of emails of p) > 0 then
                            set contactList to contactList & " <" & (value of first email of p) & ">"
                        end if
                        set contactList to contactList & linefeed
                    end repeat
                end tell
                return contactList
            "#,
                crate::sanitize_applescript(query)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Contacts tools only available on macOS".to_string())
        }
    }
}

// === CONTACTS ADDITIONS ===

/// Create a new contact
pub struct ContactsCreateTool;

#[async_trait]
impl TalosTool for ContactsCreateTool {
    fn name(&self) -> &'static str {
        "contacts_create"
    }
    fn description(&self) -> &'static str {
        "Create a new contact"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("first_name", "string", "First name", true)
            .with_param("last_name", "string", "Last name", true)
            .with_param("email", "string", "Email address", false)
            .with_param("phone", "string", "Phone number", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let first_name = args
                .get("first_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing first_name".to_string()))?;

            let last_name = args
                .get("last_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing last_name".to_string()))?;

            let escaped_first = crate::sanitize_applescript(first_name);
            let escaped_last = crate::sanitize_applescript(last_name);

            let mut extra_lines = Vec::new();

            if let Some(email) = args.get("email").and_then(|v| v.as_str()) {
                extra_lines.push(format!(
                    r#"make new email at end of emails of newPerson with properties {{label:"work", value:"{}"}}"#,
                    crate::sanitize_applescript(email)
                ));
            }

            if let Some(phone) = args.get("phone").and_then(|v| v.as_str()) {
                extra_lines.push(format!(
                    r#"make new phone at end of phones of newPerson with properties {{label:"mobile", value:"{}"}}"#,
                    crate::sanitize_applescript(phone)
                ));
            }

            let extra = extra_lines.join("\n                    ");

            let script = format!(
                r#"
                tell application "Contacts"
                    set newPerson to make new person with properties {{first name:"{}", last name:"{}"}}
                    {}
                    save
                end tell
                return "Contact created"
            "#,
                escaped_first, escaped_last, extra
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Contacts tools only available on macOS".to_string())
        }
    }
}

/// Delete a contact by name
pub struct ContactsDeleteTool;

#[async_trait]
impl TalosTool for ContactsDeleteTool {
    fn name(&self) -> &'static str {
        "contacts_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a contact by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Contact name to delete",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let script = format!(
                r#"
                tell application "Contacts"
                    set matchingPeople to (every person whose name contains "{}")
                    if (count of matchingPeople) > 0 then
                        delete (item 1 of matchingPeople)
                        save
                        return "Contact deleted"
                    else
                        return "Contact not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Contacts tools only available on macOS".to_string())
        }
    }
}

/// Update a contact
pub struct ContactsUpdateTool;

#[async_trait]
impl TalosTool for ContactsUpdateTool {
    fn name(&self) -> &'static str {
        "contacts_update"
    }
    fn description(&self) -> &'static str {
        "Update an existing contact"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Contact name to find", true)
            .with_param("email", "string", "New email address", false)
            .with_param("phone", "string", "New phone number", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let escaped_name = crate::sanitize_applescript(name);

            let mut updates = Vec::new();

            if let Some(email) = args.get("email").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"if (count of emails of p) > 0 then
                                set value of (item 1 of emails of p) to "{}"
                            else
                                make new email at end of emails of p with properties {{label:"work", value:"{}"}}
                            end if"#,
                    crate::sanitize_applescript(email),
                    crate::sanitize_applescript(email)
                ));
            }

            if let Some(phone) = args.get("phone").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"if (count of phones of p) > 0 then
                                set value of (item 1 of phones of p) to "{}"
                            else
                                make new phone at end of phones of p with properties {{label:"mobile", value:"{}"}}
                            end if"#,
                    crate::sanitize_applescript(phone),
                    crate::sanitize_applescript(phone)
                ));
            }

            if updates.is_empty() {
                return Ok("No updates specified".to_string());
            }

            let update_lines = updates.join("\n                        ");

            let script = format!(
                r#"
                tell application "Contacts"
                    set matchingPeople to (every person whose name contains "{}")
                    if (count of matchingPeople) > 0 then
                        set p to item 1 of matchingPeople
                        {}
                        save
                        return "Contact updated"
                    else
                        return "Contact not found"
                    end if
                end tell
            "#,
                escaped_name, update_lines
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Contacts tools only available on macOS".to_string())
        }
    }
}

/// List all contact groups
pub struct ContactsGroupsTool;

#[async_trait]
impl TalosTool for ContactsGroupsTool {
    fn name(&self) -> &'static str {
        "contacts_groups"
    }
    fn description(&self) -> &'static str {
        "List all contact groups"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let _ = &args;
            let script = r#"
                set groupList to ""
                tell application "Contacts"
                    repeat with g in groups
                        set groupList to groupList & (name of g) & linefeed
                    end repeat
                end tell
                if groupList is "" then
                    return "No groups found"
                end if
                return groupList
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Contacts tools only available on macOS".to_string())
        }
    }
}
