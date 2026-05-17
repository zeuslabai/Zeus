#![allow(deprecated)]
#![allow(clippy::redundant_locals)]  // Leptos signal-move pattern: `let sig = sig;`
#![allow(clippy::type_complexity)]   // Leptos closure types are inherently complex
mod api;
mod components;
mod pages;
pub mod prompt;

use leptos::prelude::*;
use leptos_router::components::*;
use leptos_router::path;

fn main() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Debug);
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| view! { <pages::not_found::NotFoundPage /> }>
                /* Standalone pages (no Shell) */
                // Marketing pages removed — served from separate zeuslab.ai repo
                <Route path=path!("/onboard") view=pages::onboard::OnboardPage />
                <Route path=path!("/onboarding") view=pages::onboarding_wizard::OnboardingWizardPage />
                <Route path=path!("/login") view=pages::login::LoginPage />
                <Route path=path!("/oauth/callback") view=pages::oauth_callback::OAuthCallbackPage />
                /* Main app with sidebar Shell */
                <ParentRoute path=path!("") view=components::layout::Shell>
                    <Route path=path!("/studio") view=pages::studio::StudioPage />
                    <Route path=path!("/chat") view=pages::studio::StudioPage />
                    <Route path=path!("") view=pages::dashboard::DashboardPage />
                    <Route path=path!("/missions") view=pages::missions::MissionsPage />
                    <Route path=path!("/missions/:id") view=pages::mission_detail::MissionDetailPage />
                    <Route path=path!("/agents") view=pages::agents::AgentsPage />
                    <Route path=path!("/agents/:id") view=pages::agent_editor::AgentEditorPage />
                    <Route path=path!("/settings") view=pages::settings::SettingsPage />
                    <Route path=path!("/skills") view=pages::skills::SkillsPage />
                    <Route path=path!("/mcp") view=pages::mcp::McpPage />
                    <Route path=path!("/memory") view=pages::memory::MemoryPage />
                    <Route path=path!("/channels") view=pages::channels::ChannelsPage />
                    <Route path=path!("/pipeline") view=pages::pipeline::PipelinePage />
                    <Route path=path!("/sessions") view=pages::sessions::SessionsPage />
                    <Route path=path!("/network") view=pages::network::NetworkPage />
                    <Route path=path!("/projects") view=pages::projects::ProjectsPage />
                    <Route path=path!("/projects/:id") view=pages::project_detail::ProjectDetailPage />
                    <Route path=path!("/security") view=pages::security::SecurityPage />
                    <Route path=path!("/analytics") view=pages::analytics::AnalyticsPage />
                    <Route path=path!("/teams") view=pages::teams::TeamsPage />
                    <Route path=path!("/extensions") view=pages::extensions::ExtensionsPage />
                    <Route path=path!("/sandbox") view=pages::sandbox::SandboxPage />
                    <Route path=path!("/approvals") view=pages::approvals::ApprovalsPage />
                    <Route path=path!("/schedules") view=pages::schedules::SchedulesPage />
                    <Route path=path!("/agora") view=pages::agora::AgoraPage />
                    <Route path=path!("/tools") view=pages::tools::ToolsPage />
                    <Route path=path!("/pantheon") view=pages::pantheon::PantheonPage />
                    <Route path=path!("/discover") view=pages::discover::DiscoverPage />
                    <Route path=path!("/goals") view=pages::goals::GoalsPage />
                    <Route path=path!("/templates") view=pages::templates::TemplatesPage />
                    <Route path=path!("/workflows") view=pages::workflows::WorkflowsPage />
                    <Route path=path!("/health") view=pages::health::HealthPage />
                    <Route path=path!("/observatory") view=pages::observatory::ObservatoryPage />
                    <Route path=path!("/voice") view=pages::voice::VoicePage />
                    <Route path=path!("/channels/telegram") view=pages::telegram::TelegramPage />
                    <Route path=path!("/channels/discord") view=pages::discord::DiscordPage />
                    <Route path=path!("/channels/slack") view=pages::slack::SlackPage />
                    <Route path=path!("/channels/whatsapp") view=pages::whatsapp::WhatsAppPage />
                    <Route path=path!("/channels/matrix") view=pages::matrix::MatrixPage />
                    <Route path=path!("/channels/imessage") view=pages::imessage::IMessagePage />
                    <Route path=path!("/channels/signal") view=pages::signal::SignalPage />
                    <Route path=path!("/channels/email") view=pages::email::EmailPage />
                    <Route path=path!("/deploy") view=pages::deploy::DeployPage />
                    <Route path=path!("/uploads") view=pages::uploads::UploadsPage />
                    <Route path=path!("/nous") view=pages::nous::NousPage />
                    <Route path=path!("/vector-stores") view=pages::vector_stores::VectorStoresPage />
                    <Route path=path!("/batch") view=pages::batch::BatchPage />
                    <Route path=path!("/reviews") view=pages::reviews::ReviewsPage />
                    <Route path=path!("/webhooks") view=pages::webhooks::WebhooksPage />
                    <Route path=path!("/ai-tools") view=pages::ai_tools::AiToolsPage />
                    <Route path=path!("/canvas") view=pages::canvas::CanvasPage />
                    <Route path=path!("/office") view=pages::office::OfficePage />
                    // blog-admin removed — part of zeuslab.ai marketing site
                </ParentRoute>
            </Routes>
        </Router>
    }
}
