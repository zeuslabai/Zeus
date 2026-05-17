// ═══════════════════════════════════════════════════════════
// ZEUS — Uploads Page — File upload with drag-and-drop + extraction preview
// PDF/DOCX text extraction, upload history, metadata viewer
// ═══════════════════════════════════════════════════════════

use crate::api;
use crate::components::design::*;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

fn file_icon(mime: &str, ext: &str) -> &'static str {
    match (mime, ext) {
        (m, _) if m.contains("pdf") => "\u{1f4c4}",
        (_, "docx") | (_, "doc") => "\u{1f4dd}",
        (m, _) if m.starts_with("image/") => "\u{1f5bc}\u{fe0f}",
        (_, "md") | (_, "txt") => "\u{1f4c3}",
        (_, "xlsx") | (_, "xls") | (_, "csv") => "\u{1f4ca}",
        _ => "\u{1f4ce}",
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[component]
pub fn UploadsPage() -> impl IntoView {
    let files = RwSignal::new(Vec::<api::UploadedFile>::new());
    let loading = RwSignal::new(true);
    let uploading = RwSignal::new(false);
    let upload_progress = RwSignal::new(String::new());
    let toast_msg = RwSignal::new(String::new());
    let toast_ok = RwSignal::new(true);
    let dragging = RwSignal::new(false);
    let preview_file = RwSignal::new(Option::<api::UploadedFile>::None);

    // Load existing uploads
    let reload = move || {
        spawn_local(async move {
            match api::list_uploads().await {
                Ok(f) => files.set(f),
                Err(e) => {
                    toast_ok.set(false);
                    toast_msg.set(format!("Load error: {}", e));
                }
            }
            loading.set(false);
        });
    };
    reload();

    // Upload a file via web_sys::File
    let do_upload = move |file: web_sys::File| {
        let name = file.name();
        uploading.set(true);
        upload_progress.set(format!("Uploading {}...", name));
        spawn_local(async move {
            match api::upload_file(file, |_| {}).await {
                Ok(uploaded) => {
                    // Fetch full metadata (includes extracted_text)
                    let full = api::get_upload_metadata(&uploaded.id)
                        .await
                        .unwrap_or(uploaded);
                    toast_ok.set(true);
                    toast_msg.set(format!("Uploaded: {}", full.name));
                    // Show extraction preview if text was extracted
                    if full.extracted_text.is_some() {
                        preview_file.set(Some(full));
                    }
                    // Reload list
                    if let Ok(f) = api::list_uploads().await {
                        files.set(f);
                    }
                }
                Err(e) => {
                    toast_ok.set(false);
                    toast_msg.set(format!("Upload failed: {}", e));
                }
            }
            uploading.set(false);
            upload_progress.set(String::new());
        });
    };

    // Delete handler
    let do_delete = move |id: String, name: String| {
        spawn_local(async move {
            match api::delete_upload(&id).await {
                Ok(()) => {
                    toast_ok.set(true);
                    toast_msg.set(format!("Deleted: {}", name));
                    if let Ok(f) = api::list_uploads().await {
                        files.set(f);
                    }
                    // Clear preview if deleted file was being previewed
                    if preview_file.get_untracked().as_ref().map(|f| f.id.as_str())
                        == Some(id.as_str())
                    {
                        preview_file.set(None);
                    }
                }
                Err(e) => {
                    toast_ok.set(false);
                    toast_msg.set(format!("Delete failed: {}", e));
                }
            }
        });
    };

    // View metadata/extraction for a file
    let do_preview = move |id: String| {
        spawn_local(async move {
            match api::get_upload_metadata(&id).await {
                Ok(f) => preview_file.set(Some(f)),
                Err(e) => {
                    toast_ok.set(false);
                    toast_msg.set(format!("Metadata error: {}", e));
                }
            }
        });
    };

    // Handle file input change
    let on_file_input = move |ev: web_sys::Event| {
        let target: web_sys::HtmlInputElement = ev.target().unwrap().dyn_into().unwrap();
        if let Some(file_list) = target.files()
            && let Some(file) = file_list.get(0) {
                do_upload(file);
            }
        // Reset input so same file can be re-selected
        target.set_value("");
    };

    view! {
        <div style="padding: 32px; display: flex; gap: 24px;">
            // Main column
            <div style="flex: 1; min-width: 0;">
                // Toast
                <Show when=move || !toast_msg.get().is_empty()>
                    <div style=move || format!(
                        "position: fixed; top: 20px; right: 20px; z-index: 9999; padding: 12px 20px; border-radius: 10px; \
                        font-size: 13px; font-family: 'Rajdhani', sans-serif; \
                        background: {}; border: 1px solid {}; color: rgba(255,245,240,0.9); \
                        box-shadow: 0 4px 20px rgba(0,0,0,0.4); cursor: pointer;",
                        if toast_ok.get() { "rgba(34,197,94,0.15)" } else { "rgba(239,68,68,0.15)" },
                        if toast_ok.get() { "rgba(34,197,94,0.3)" } else { "rgba(239,68,68,0.3)" },
                    ) on:click=move |_| toast_msg.set(String::new())>
                        {move || toast_msg.get()}
                    </div>
                </Show>

                // Header
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                    <div>
                        <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">
                            "\u{1f4ce} UPLOADS"
                        </h1>
                        <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                            {move || {
                                let f = files.get();
                                let total_size: u64 = f.iter().map(|f| f.size).sum();
                                if loading.get() { "Loading...".to_string() }
                                else if f.is_empty() { "No files uploaded".to_string() }
                                else { format!("{} file{} \u{00b7} {}", f.len(), if f.len() == 1 { "" } else { "s" }, format_size(total_size)) }
                            }}
                        </p>
                    </div>
                    <div style="display: flex; gap: 8px;">
                        <label style="cursor: pointer; display: inline-block;">
                            <input type="file" style="display: none;"
                                accept=".pdf,.docx,.doc,.txt,.md,.xlsx,.xls,.csv,.png,.jpg,.jpeg,.gif,.webp"
                                on:change=on_file_input
                            />
                            <div style="padding: 8px 16px; border-radius: 8px; background: rgba(255,60,20,0.8); color: rgba(255,245,240,0.95); font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; font-weight: 700; cursor: pointer;">
                                "+ UPLOAD FILE"
                            </div>
                        </label>
                    </div>
                </div>

                // Drag-and-drop zone
                <div
                    style=move || format!(
                        "border: 2px dashed {}; border-radius: 16px; padding: {}; text-align: center; \
                        margin-bottom: 24px; transition: all 0.2s; cursor: pointer; background: {};",
                        if dragging.get() { "rgba(255,60,20,0.6)" } else { "rgba(255,245,240,0.08)" },
                        if uploading.get() { "20px 32px" } else { "32px" },
                        if dragging.get() { "rgba(255,60,20,0.06)" } else { "transparent" },
                    )
                    on:dragover=move |ev: web_sys::DragEvent| {
                        ev.prevent_default();
                        dragging.set(true);
                    }
                    on:dragleave=move |_| dragging.set(false)
                    on:drop=move |ev: web_sys::DragEvent| {
                        ev.prevent_default();
                        dragging.set(false);
                        if let Some(dt) = ev.data_transfer()
                            && let Some(file_list) = dt.files()
                                && let Some(file) = file_list.get(0) {
                                    do_upload(file);
                                }
                    }
                >
                    {move || if uploading.get() {
                        view! {
                            <div style="display: flex; align-items: center; justify-content: center; gap: 12px;">
                                <div style="width: 20px; height: 20px; border: 2px solid rgba(255,60,20,0.3); border-top-color: rgba(255,60,20,0.8); border-radius: 50%; animation: spin 0.8s linear infinite;" />
                                <span style="font-size: 14px; color: rgba(255,245,240,0.8); font-family: 'Rajdhani', sans-serif;">
                                    {upload_progress.get()}
                                </span>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div>
                                <div style="font-size: 36px; margin-bottom: 8px; opacity: 0.4;">"\u{2601}\u{fe0f}"</div>
                                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">
                                    "DROP FILES HERE"
                                </div>
                                <div style="font-size: 12px; color: rgba(255,245,240,0.35);">
                                    "PDF, DOCX, TXT, MD, images, spreadsheets"
                                </div>
                            </div>
                        }.into_any()
                    }}
                </div>

                // File list
                <Show when=move || loading.get()>
                    <div style="text-align: center; padding: 40px 0; color: rgba(255,245,240,0.3); font-size: 13px;">
                        "Loading uploads..."
                    </div>
                </Show>

                <Show when=move || !loading.get() && files.get().is_empty()>
                    <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 200px; gap: 12px;">
                        <div style="width: 56px; height: 56px; border-radius: 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center;">
                            <Icon name="tools" size=24 color="rgba(255,60,20,0.6)".to_string() />
                        </div>
                        <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9);">
                            "NO FILES YET"
                        </div>
                        <div style="font-size: 13px; color: rgba(255,245,240,0.7); max-width: 360px; text-align: center; line-height: 1.6;">
                            "Upload PDFs and documents to extract text, or images for thumbnails."
                        </div>
                    </div>
                </Show>

                <Show when=move || !files.get().is_empty()>
                    <div style="display: flex; flex-direction: column; gap: 8px;">
                        {move || files.get().into_iter().map(|f| {
                            let fid = f.id.clone();
                            let fid2 = f.id.clone();
                            let fid_dl = f.id.clone();
                            let fid_del = f.id.clone();
                            let fname_del = f.name.clone();
                            let has_text = f.extracted_text.is_some();
                            let has_thumb = f.thumbnail_url.is_some();
                            let ext = f.extension.clone();
                            let mime = f.mime_type.clone();
                            let do_del = do_delete;
                            view! {
                                <Card>
                                    <div style="display: flex; align-items: center; gap: 14px;">
                                        // Icon / Thumbnail
                                        {if has_thumb {
                                            let thumb_url = f.thumbnail_url.clone().unwrap_or_default();
                                            view! {
                                                <img src=thumb_url
                                                    style="width: 44px; height: 44px; border-radius: 8px; object-fit: cover; border: 1px solid rgba(255,245,240,0.06);"
                                                />
                                            }.into_any()
                                        } else {
                                            view! {
                                                <div style="width: 44px; height: 44px; border-radius: 8px; background: rgba(255,60,20,0.08); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center; font-size: 22px;">
                                                    {file_icon(&mime, &ext)}
                                                </div>
                                            }.into_any()
                                        }}

                                        // File info
                                        <div style="flex: 1; min-width: 0;">
                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                <span
                                                    style="font-family: 'Rajdhani', sans-serif; font-size: 14px; color: rgba(255,245,240,0.95); font-weight: 600; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;"
                                                    on:click=move |_| do_preview(fid.clone())
                                                >{f.name.clone()}</span>
                                                {has_text.then(|| view! {
                                                    <span style="font-size: 8px; padding: 1px 6px; border-radius: 4px; background: rgba(34,197,94,0.12); border: 1px solid rgba(34,197,94,0.2); color: rgba(34,197,94,0.8); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                        "EXTRACTED"
                                                    </span>
                                                })}
                                            </div>
                                            <div style="display: flex; gap: 12px; font-size: 11px; color: rgba(255,245,240,0.35); margin-top: 2px;">
                                                <span>{format_size(f.size)}</span>
                                                <span>{f.mime_type.clone()}</span>
                                                {(!f.uploaded_at.is_empty()).then(|| view! {
                                                    <span>{f.uploaded_at.get(..16).unwrap_or(&f.uploaded_at).to_string()}</span>
                                                })}
                                            </div>
                                        </div>

                                        // Actions
                                        <div style="display: flex; gap: 6px;">
                                            <button
                                                style="padding: 5px 12px; border-radius: 6px; font-size: 10px; font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer; background: rgba(255,60,20,0.1); border: 1px solid rgba(255,60,20,0.2); color: rgba(255,140,80,0.9);"
                                                on:click=move |_| do_preview(fid2.clone())
                                            >"View"</button>
                                            <a href=format!("/v1/uploads/{}/download", fid_dl)
                                                style="padding: 5px 12px; border-radius: 6px; font-size: 10px; font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer; background: transparent; border: 1px solid rgba(255,245,240,0.08); color: rgba(255,245,240,0.5); text-decoration: none; display: inline-flex; align-items: center;"
                                                download=""
                                            >"DL"</a>
                                            <button
                                                style="padding: 5px 8px; border-radius: 6px; font-size: 10px; font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer; background: rgba(239,68,68,0.06); border: 1px solid rgba(239,68,68,0.12); color: rgba(239,68,68,0.5);"
                                                on:click=move |_| do_del(fid_del.clone(), fname_del.clone())
                                            >"DEL"</button>
                                        </div>
                                    </div>
                                </Card>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </Show>
            </div>

            // Extraction preview panel (right side)
            <Show when=move || preview_file.get().is_some()>
                <div style="width: 420px; flex-shrink: 0;">
                    {move || {
                        let pf = match preview_file.get() {
                            Some(f) => f,
                            None => return view! { <div /> }.into_any(),
                        };
                        let has_text = pf.extracted_text.is_some();
                        let text = pf.extracted_text.clone().unwrap_or_default();
                        let text_len = text.len();
                        view! {
                            <Card>
                                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.9);">
                                        "FILE DETAILS"
                                    </div>
                                    <button
                                        style="background: none; border: 1px solid rgba(255,60,20,0.2); color: rgba(255,245,240,0.5); padding: 3px 8px; border-radius: 4px; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px;"
                                        on:click=move |_| preview_file.set(None)
                                    >"CLOSE"</button>
                                </div>

                                // File meta
                                <div style="display: flex; flex-direction: column; gap: 8px; margin-bottom: 16px;">
                                    <div style="display: flex; gap: 8px;">
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.35); width: 60px;">"Name:"</span>
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.8); word-break: break-all;">{pf.name.clone()}</span>
                                    </div>
                                    <div style="display: flex; gap: 8px;">
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.35); width: 60px;">"Size:"</span>
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.8);">{format_size(pf.size)}</span>
                                    </div>
                                    <div style="display: flex; gap: 8px;">
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.35); width: 60px;">"Type:"</span>
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.8);">{pf.mime_type.clone()}</span>
                                    </div>
                                    {(!pf.uploaded_at.is_empty()).then(|| view! {
                                        <div style="display: flex; gap: 8px;">
                                            <span style="font-size: 11px; color: rgba(255,245,240,0.35); width: 60px;">"Date:"</span>
                                            <span style="font-size: 11px; color: rgba(255,245,240,0.8);">{pf.uploaded_at.clone()}</span>
                                        </div>
                                    })}
                                    <div style="display: flex; gap: 8px;">
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.35); width: 60px;">"ID:"</span>
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.5); font-family: monospace; font-size: 10px;">{pf.id.clone()}</span>
                                    </div>
                                </div>

                                // Thumbnail for images
                                {pf.thumbnail_url.as_ref().map(|url| {
                                    let url = url.clone();
                                    view! {
                                        <div style="margin-bottom: 16px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.4); margin-bottom: 6px;">
                                                "PREVIEW"
                                            </div>
                                            <img src=url
                                                style="max-width: 100%; border-radius: 8px; border: 1px solid rgba(255,245,240,0.06);"
                                            />
                                        </div>
                                    }
                                })}

                                // Extracted text
                                {if has_text {
                                    view! {
                                        <div>
                                            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.4);">
                                                    "EXTRACTED TEXT"
                                                </div>
                                                <span style="font-size: 10px; color: rgba(255,245,240,0.3);">
                                                    {format!("{} chars", text_len)}
                                                </span>
                                            </div>
                                            <div style="max-height: 500px; overflow-y: auto; padding: 12px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.08); border-radius: 8px; font-size: 12px; color: rgba(255,245,240,0.75); line-height: 1.6; font-family: 'Rajdhani', sans-serif; white-space: pre-wrap; word-break: break-word;">
                                                {text}
                                            </div>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <div style="padding: 20px; text-align: center; color: rgba(255,245,240,0.3); font-size: 12px;">
                                            "No extracted text available for this file type."
                                        </div>
                                    }.into_any()
                                }}
                            </Card>
                        }.into_any()
                    }}
                </div>
            </Show>
        </div>
    }
}
