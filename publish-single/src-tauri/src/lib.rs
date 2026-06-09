use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

const DATA_FILE: &str = "publish-single-data.json";

#[derive(Clone, Default)]
struct AppState {
    controls: Arc<Mutex<HashMap<String, Arc<TaskControl>>>>,
}

#[derive(Default)]
struct TaskControl {
    paused: AtomicBool,
    stopped: AtomicBool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleConfig {
    pub ai1_provider_id: String,
    pub ai2_provider_id: String,
    pub ai1_system_prompt: String,
    pub ai2_system_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DisplayMode {
    Dual,
    Timeline,
    Focus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SuccessMode {
    Ai1Judgement,
    Manual,
    Keyword,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub providers: Vec<ProviderConfig>,
    pub role_config: RoleConfig,
    pub templates: Vec<PromptTemplate>,
    pub active_template_id: String,
    #[serde(default)]
    pub ai2_local_paths: Vec<String>,
    pub display_mode: DisplayMode,
    pub success_mode: SuccessMode,
    pub success_keywords: Vec<String>,
    pub theme: ThemeMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Running,
    Paused,
    Stopped,
    Succeeded,
    Failed,
    LimitReached,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RoundStatus {
    Completed,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Round {
    pub index: u32,
    pub ai1_plan: String,
    pub ai2_prompt: String,
    pub ai2_result: String,
    pub ai1_review: String,
    pub status: RoundStatus,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub user_goal: String,
    pub max_rounds: u32,
    pub status: TaskStatus,
    pub success_mode: SuccessMode,
    pub display_mode: DisplayMode,
    pub rounds: Vec<Round>,
    pub final_result: String,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTaskRequest {
    pub user_goal: String,
    pub max_rounds: u32,
    pub settings: Settings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestProviderRequest {
    pub provider: ProviderConfig,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredData {
    settings: Settings,
    tasks: Vec<TaskRecord>,
}

impl Default for StoredData {
    fn default() -> Self {
        Self {
            settings: default_settings(),
            tasks: Vec::new(),
        }
    }
}

#[tauri::command]
fn load_settings(app: AppHandle) -> Result<Settings, String> {
    Ok(load_data(&app)?.settings)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let mut data = load_data(&app)?;
    data.settings = settings.clone();
    save_data(&app, &data)?;
    Ok(settings)
}

#[tauri::command]
fn list_tasks(app: AppHandle) -> Result<Vec<TaskRecord>, String> {
    let mut tasks = load_data(&app)?.tasks;
    tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(tasks)
}

#[tauri::command]
fn get_task(app: AppHandle, task_id: String) -> Result<Option<TaskRecord>, String> {
    Ok(load_data(&app)?.tasks.into_iter().find(|task| task.id == task_id))
}

#[tauri::command]
async fn start_task(
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartTaskRequest,
) -> Result<TaskRecord, String> {
    let now = timestamp();
    let mut data = load_data(&app)?;
    data.settings = request.settings.clone();

    let mut task = TaskRecord {
        id: Uuid::new_v4().to_string(),
        user_goal: request.user_goal.trim().to_string(),
        max_rounds: request.max_rounds,
        status: TaskStatus::Running,
        success_mode: request.settings.success_mode.clone(),
        display_mode: request.settings.display_mode.clone(),
        rounds: Vec::new(),
        final_result: String::new(),
        error: None,
        created_at: now.clone(),
        updated_at: now,
    };

    data.tasks.push(task.clone());
    save_data(&app, &data)?;
    emit_task(&app, &task);

    let control = Arc::new(TaskControl::default());
    state
        .controls
        .lock()
        .map_err(|_| "任务控制状态已损坏".to_string())?
        .insert(task.id.clone(), Arc::clone(&control));

    let runner_app = app.clone();
    let runner_request = request.clone();
    let runner_control = Arc::clone(&control);
    let runner_controls = Arc::clone(&state.controls);
    let mut runner_task = task.clone();

    tauri::async_runtime::spawn(async move {
        let result =
            run_task_loop(&runner_app, &runner_request, &mut runner_task, runner_control).await;

        if let Err(error) = result {
            runner_task.status = TaskStatus::Failed;
            runner_task.error = Some(error);
            runner_task.updated_at = timestamp();
            let _ = upsert_task(&runner_app, &runner_task);
            emit_task(&runner_app, &runner_task);
        }

        if let Ok(mut controls) = runner_controls.lock() {
            controls.remove(&runner_task.id);
        }
    });

    Ok(task)
}

#[tauri::command]
fn pause_task(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    if let Some(control) = state
        .controls
        .lock()
        .map_err(|_| "任务控制状态已损坏".to_string())?
        .get(&task_id)
    {
        control.paused.store(true, Ordering::SeqCst);
    }
    update_task_status(&app, &task_id, TaskStatus::Paused)
}

#[tauri::command]
fn resume_task(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    if let Some(control) = state
        .controls
        .lock()
        .map_err(|_| "任务控制状态已损坏".to_string())?
        .get(&task_id)
    {
        control.paused.store(false, Ordering::SeqCst);
    }
    update_task_status(&app, &task_id, TaskStatus::Running)
}

#[tauri::command]
fn stop_task(app: AppHandle, state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    if let Some(control) = state
        .controls
        .lock()
        .map_err(|_| "任务控制状态已损坏".to_string())?
        .get(&task_id)
    {
        control.stopped.store(true, Ordering::SeqCst);
        control.paused.store(false, Ordering::SeqCst);
    }
    update_task_status(&app, &task_id, TaskStatus::Stopped)
}

#[tauri::command]
fn delete_task(app: AppHandle, task_id: String) -> Result<(), String> {
    let mut data = load_data(&app)?;
    data.tasks.retain(|task| task.id != task_id);
    save_data(&app, &data)
}

#[tauri::command]
async fn test_provider_connection(request: TestProviderRequest) -> Result<String, String> {
    let response = call_chat(
        &request.provider,
        &request.system_prompt,
        "请只回复 OK，用于测试模型连接。",
    )
    .await?;

    Ok(format!("连接成功：{}", response.chars().take(80).collect::<String>()))
}

async fn run_task_loop(
    app: &AppHandle,
    request: &StartTaskRequest,
    task: &mut TaskRecord,
    control: Arc<TaskControl>,
) -> Result<(), String> {
    if task.user_goal.is_empty() {
        return Err("用户目标不能为空".to_string());
    }

    let ai1 = find_provider(&request.settings, &request.settings.role_config.ai1_provider_id)?;
    let ai2 = find_provider(&request.settings, &request.settings.role_config.ai2_provider_id)?;
    let template = find_template(&request.settings)?;
    let mut previous_ai2_result = String::new();
    let mut round_index = 1;

    while should_run_round(round_index, task.max_rounds) {
        wait_if_paused(app, task, &control).await?;
        if control.stopped.load(Ordering::SeqCst) {
            finish_task(app, task, TaskStatus::Stopped, "用户已停止任务")?;
            return Ok(());
        }

        let started = Instant::now();
        let ai1_prompt = build_ai1_prompt(&task.user_goal, round_index, &previous_ai2_result);
        let ai1_plan = call_chat(
            ai1,
            &request.settings.role_config.ai1_system_prompt,
            &ai1_prompt,
        )
        .await?;

        wait_if_paused(app, task, &control).await?;
        if control.stopped.load(Ordering::SeqCst) {
            finish_task(app, task, TaskStatus::Stopped, "用户已停止任务")?;
            return Ok(());
        }

        let local_context = build_local_file_context(&request.settings.ai2_local_paths)?;
        let ai2_prompt = compose_ai2_prompt(template, &ai1_plan, &local_context);
        let raw_ai2_result = run_ai2_tool_agent(
            ai2,
            &request.settings.role_config.ai2_system_prompt,
            &ai2_prompt,
            &request.settings.ai2_local_paths,
        )
        .await?;
        let write_summary =
            apply_ai2_file_writes(&raw_ai2_result, &request.settings.ai2_local_paths)?;
        let ai2_result = append_write_summary(raw_ai2_result, write_summary);

        let review_prompt = build_review_prompt(&task.user_goal, round_index, &ai2_result);
        let ai1_review = call_chat(
            ai1,
            &request.settings.role_config.ai1_system_prompt,
            &review_prompt,
        )
        .await?;

        let succeeded = detect_success(
            &task.success_mode,
            &request.settings.success_keywords,
            &ai2_result,
            &ai1_review,
        );

        let round = Round {
            index: round_index,
            ai1_plan,
            ai2_prompt,
            ai2_result: ai2_result.clone(),
            ai1_review: ai1_review.clone(),
            status: if succeeded {
                RoundStatus::Succeeded
            } else {
                RoundStatus::Completed
            },
            duration_ms: started.elapsed().as_millis(),
        };

        task.rounds.push(round);
        task.final_result = if succeeded { ai2_result.clone() } else { ai1_review };
        task.status = if succeeded {
            TaskStatus::Succeeded
        } else {
            TaskStatus::Running
        };
        task.updated_at = timestamp();
        upsert_task(app, task)?;
        emit_task(app, task);

        if succeeded {
            return Ok(());
        }

        previous_ai2_result = ai2_result;
        round_index += 1;
    }

    finish_task(app, task, TaskStatus::LimitReached, "已达到最大轮数")?;
    Ok(())
}

async fn wait_if_paused(
    app: &AppHandle,
    task: &mut TaskRecord,
    control: &TaskControl,
) -> Result<(), String> {
    let mut emitted_pause = false;
    while control.paused.load(Ordering::SeqCst) {
        if control.stopped.load(Ordering::SeqCst) {
            return Ok(());
        }
        if !emitted_pause {
            task.status = TaskStatus::Paused;
            task.updated_at = timestamp();
            upsert_task(app, task)?;
            emit_task(app, task);
            emitted_pause = true;
        }
        sleep(Duration::from_millis(250)).await;
    }
    if emitted_pause {
        task.status = TaskStatus::Running;
        task.updated_at = timestamp();
        upsert_task(app, task)?;
        emit_task(app, task);
    }
    Ok(())
}

async fn call_chat(
    provider: &ProviderConfig,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    call_chat_messages(
        provider,
        &[
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
            },
        ],
    )
    .await
}

async fn call_chat_messages(
    provider: &ProviderConfig,
    messages: &[ChatMessage],
) -> Result<String, String> {
    if provider.base_url.trim().is_empty() {
        return Err(format!("{} 的 base URL 不能为空", provider.name));
    }
    if provider.api_key.trim().is_empty() {
        return Err(format!("{} 的 API key 不能为空", provider.name));
    }
    if provider.model.trim().is_empty() {
        return Err(format!("{} 的 model 不能为空", provider.name));
    }

    let client = reqwest::Client::new();
    let response = client
        .post(chat_endpoint(&provider.base_url))
        .bearer_auth(provider.api_key.trim())
        .json(&json!({
            "model": provider.model,
            "temperature": provider.temperature,
            "messages": messages
        }))
        .send()
        .await
        .map_err(|error| format!("请求 {} 失败: {error}", provider.name))?;

    let status = response.status();
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("解析 {} 响应失败: {error}", provider.name))?;

    if !status.is_success() {
        return Err(format!("{} 返回错误 {status}: {value}", provider.name));
    }

    value["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| value["choices"][0]["text"].as_str())
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| format!("{} 响应中没有可用文本: {value}", provider.name))
}

async fn run_ai2_tool_agent(
    provider: &ProviderConfig,
    system_prompt: &str,
    user_prompt: &str,
    allowed_paths: &[String],
) -> Result<String, String> {
    let tool_prompt = format!("{system_prompt}\n\n{}", ai2_tool_instructions());
    let mut messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: tool_prompt,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_prompt.to_string(),
        },
    ];
    let mut tool_log = Vec::new();

    for _ in 0..8 {
        let response = call_chat_messages(provider, &messages).await?;
        let calls = parse_tool_calls(&response);
        if calls.is_empty() {
            if tool_log.is_empty() {
                return Ok(response);
            }
            return Ok(format!(
                "{response}\n\n[AI2 工具调用记录]\n{}",
                tool_log.join("\n\n")
            ));
        }

        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: response,
        });

        let mut result_chunks = Vec::new();
        for call in calls {
            let result = execute_ai2_tool(&call, allowed_paths);
            let rendered = match result {
                Ok(output) => format!("tool={} status=ok\n{}", call.name, output),
                Err(error) => format!("tool={} status=error\n{}", call.name, error),
            };
            tool_log.push(rendered.clone());
            result_chunks.push(rendered);
        }

        messages.push(ChatMessage {
            role: "user".to_string(),
            content: format!(
                "工具执行结果如下。请基于结果继续；如果还需要文件信息，可以继续调用工具；如果已经足够，请输出最终答案，不要再输出工具调用块。\n\n{}",
                result_chunks.join("\n\n---\n\n")
            ),
        });
    }

    Ok(format!(
        "AI2 工具调用达到上限，已停止继续调用。\n\n[AI2 工具调用记录]\n{}",
        tool_log.join("\n\n")
    ))
}

fn load_data(app: &AppHandle) -> Result<StoredData, String> {
    let path = data_path(app)?;
    read_data_from_path(&path)
}

fn save_data(app: &AppHandle, data: &StoredData) -> Result<(), String> {
    let path = data_path(app)?;
    write_data_to_path(&path, data)
}

fn data_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法定位应用数据目录: {error}"))?;
    Ok(dir.join(DATA_FILE))
}

fn read_data_from_path(path: &Path) -> Result<StoredData, String> {
    if !path.exists() {
        return Ok(StoredData::default());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("读取数据文件失败 {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("解析数据文件失败 {}: {error}", path.display()))
}

fn write_data_to_path(path: &Path, data: &StoredData) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建数据目录失败 {}: {error}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(data)
        .map_err(|error| format!("序列化数据失败: {error}"))?;
    fs::write(path, content)
        .map_err(|error| format!("写入数据文件失败 {}: {error}", path.display()))
}

fn upsert_task(app: &AppHandle, task: &TaskRecord) -> Result<(), String> {
    let mut data = load_data(app)?;
    match data.tasks.iter_mut().find(|item| item.id == task.id) {
        Some(existing) => *existing = task.clone(),
        None => data.tasks.push(task.clone()),
    }
    save_data(app, &data)
}

fn update_task_status(app: &AppHandle, task_id: &str, status: TaskStatus) -> Result<(), String> {
    let mut data = load_data(app)?;
    if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
        task.status = status;
        task.updated_at = timestamp();
        let task = task.clone();
        save_data(app, &data)?;
        emit_task(app, &task);
    }
    Ok(())
}

fn finish_task(
    app: &AppHandle,
    task: &mut TaskRecord,
    status: TaskStatus,
    final_result: &str,
) -> Result<(), String> {
    task.status = status;
    task.final_result = final_result.to_string();
    task.updated_at = timestamp();
    upsert_task(app, task)?;
    emit_task(app, task);
    Ok(())
}

fn emit_task(app: &AppHandle, task: &TaskRecord) {
    let _ = app.emit("task-updated", task.clone());
}

fn find_provider<'a>(settings: &'a Settings, id: &str) -> Result<&'a ProviderConfig, String> {
    settings
        .providers
        .iter()
        .find(|provider| provider.id == id)
        .ok_or_else(|| format!("找不到供应商配置: {id}"))
}

fn find_template(settings: &Settings) -> Result<&PromptTemplate, String> {
    settings
        .templates
        .iter()
        .find(|template| template.id == settings.active_template_id)
        .or_else(|| settings.templates.first())
        .ok_or_else(|| "至少需要一个提示词模板".to_string())
}

fn build_ai1_prompt(user_goal: &str, round_index: u32, previous_ai2_result: &str) -> String {
    if round_index == 1 {
        format!(
            "用户目标:\n{user_goal}\n\n请分析需求，给出可以直接交给 AI2 执行的方案。"
        )
    } else {
        format!(
            "用户目标:\n{user_goal}\n\n上一轮 AI2 执行结果:\n{previous_ai2_result}\n\n请审核上一轮结果，给出下一轮可以直接交给 AI2 执行的改进方案。"
        )
    }
}

fn build_review_prompt(user_goal: &str, round_index: u32, ai2_result: &str) -> String {
    format!(
        "用户目标:\n{user_goal}\n\n第 {round_index} 轮 AI2 执行结果:\n{ai2_result}\n\n请审核结果是否已经达到用户目标。如果达到，请明确写出“成功”；如果没有达到，请明确写出“继续”并说明下一步。"
    )
}

fn compose_ai2_prompt(template: &PromptTemplate, ai1_content: &str, local_context: &str) -> String {
    [
        template.prefix.as_str(),
        ai1_content,
        local_context,
        template.suffix.as_str(),
    ]
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_local_file_context(paths: &[String]) -> Result<String, String> {
    let normalized = normalize_allowed_paths(paths)?;
    if normalized.is_empty() {
        return Ok(String::new());
    }

    let mut sections = vec![
        "AI2 本地文件权限已启用：下面的“本地文件上下文”是后端已经从用户授权路径读取到的真实文件信息。不要再声称没有文件读取能力；如果信息不足，只能说明缺少哪一类进一步解析能力。若需要改写文件，请在回复中输出一个或多个写入块：\n<write_file path=\"绝对路径\">\n文件完整内容\n</write_file>\n后端只会写入授权路径内的文件。".to_string(),
    ];

    for path in normalized {
        if path.is_file() {
            sections.push(summarize_file(&path)?);
        } else if path.is_dir() {
            let entries = fs::read_dir(&path)
                .map_err(|error| format!("读取目录失败 {}: {error}", path.display()))?
                .take(50)
                .map(|entry| {
                    entry
                        .map(|item| {
                            let item_path = item.path();
                            if item_path.is_file() {
                                summarize_file(&item_path)
                            } else if item_path.is_dir() {
                                Ok(format!("[目录] {}", item_path.display()))
                            } else {
                                Ok(format!("[其他] {}", item_path.display()))
                            }
                        })
                        .map_err(|error| error.to_string())?
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("读取目录条目失败 {}: {error}", path.display()))?;
            sections.push(format!(
                "授权目录: {}\n{}",
                path.display(),
                entries.join("\n")
            ));
        } else {
            return Err(format!("授权路径不存在: {}", path.display()));
        }
    }

    Ok(sections.join("\n\n"))
}

fn summarize_file(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("读取文件元数据失败 {}: {error}", path.display()))?;
    let bytes = fs::read(path).map_err(|error| format!("读取文件失败 {}: {error}", path.display()))?;
    let sha256 = Sha256::digest(&bytes);
    let modified = metadata
        .modified()
        .ok()
        .map(|time| {
            let datetime: chrono::DateTime<chrono::Utc> = time.into();
            datetime.to_rfc3339()
        })
        .unwrap_or_else(|| "unknown".to_string());
    let preview = file_preview(&bytes);

    Ok(format!(
        "[文件] {}\n大小: {} bytes\n修改时间: {}\nSHA256: {:x}\n{}",
        path.display(),
        metadata.len(),
        modified,
        sha256,
        preview
    ))
}

fn file_preview(bytes: &[u8]) -> String {
    const MAX_FILE_BYTES: usize = 60 * 1024;
    let truncated = bytes.len() > MAX_FILE_BYTES;
    let slice = if truncated {
        &bytes[..MAX_FILE_BYTES]
    } else {
        &bytes
    };
    if let Ok(text) = std::str::from_utf8(slice) {
        if is_probably_text(text) {
            let mut content = text.to_string();
            if truncated {
                content.push_str("\n\n[文件过大，已截断前 60KB]");
            }
            return format!("文本预览:\n```text\n{}\n```", content);
        }
    }

    let hex = bytes
        .iter()
        .take(1024)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .chunks(16)
        .map(|chunk| chunk.join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    let suffix = if bytes.len() > 1024 {
        "\n[二进制文件，仅显示前 1024 bytes]"
    } else {
        ""
    };
    format!("十六进制预览:\n```hex\n{}{}\n```", hex, suffix)
}

fn is_probably_text(text: &str) -> bool {
    let sample_len = text.chars().take(1024).count().max(1);
    let control_count = text
        .chars()
        .take(1024)
        .filter(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        .count();
    control_count * 100 / sample_len < 5
}

fn ai2_tool_instructions() -> &'static str {
    r#"AI2 文件工具能力已启用。你可以像 Codex 一样按需读取、搜索、分析和改写用户授权路径内的文件。

可用工具：
- list_dir: {"path":"绝对路径"}
- read_file: {"path":"绝对路径","offset":0,"length":60000}
- write_file: {"path":"绝对路径","content":"完整文件内容"}
- file_info: {"path":"绝对路径"}
- search_files: {"root":"绝对路径","query":"文件名关键词"}
- grep: {"root":"绝对路径","query":"文本关键词"}
- analyze_pe_basic: {"path":"DLL或EXE绝对路径"}

调用工具时，只输出工具块：
<tool_call name="工具名">
{"path":"D:\\example\\file.txt"}
</tool_call>

工具结果会由后端返回给你。拿到足够信息后，输出最终答案，不要再说“没有文件读取能力”。所有工具都只能访问用户在设置中授权的路径。"#
}

#[derive(Debug)]
struct Ai2ToolCall {
    name: String,
    args: serde_json::Value,
}

fn parse_tool_calls(input: &str) -> Vec<Ai2ToolCall> {
    let mut calls = Vec::new();
    let mut rest = input;
    let start_marker = "<tool_call name=\"";
    let end_marker = "</tool_call>";

    while let Some(start) = rest.find(start_marker) {
        let after_start = &rest[start + start_marker.len()..];
        let Some(name_end) = after_start.find("\">") else {
            break;
        };
        let name = after_start[..name_end].trim().to_string();
        let after_name = &after_start[name_end + 2..];
        let Some(args_end) = after_name.find(end_marker) else {
            break;
        };
        let args_text = after_name[..args_end].trim();
        if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_text) {
            calls.push(Ai2ToolCall { name, args });
        }
        rest = &after_name[args_end + end_marker.len()..];
    }

    calls
}

fn execute_ai2_tool(call: &Ai2ToolCall, allowed_paths: &[String]) -> Result<String, String> {
    let allowed = normalize_allowed_paths(allowed_paths)?;
    if allowed.is_empty() {
        return Err("用户还没有授权任何本地路径。请先在设置中选择文件或目录。".to_string());
    }

    match call.name.as_str() {
        "list_dir" => tool_list_dir(arg_path(&call.args, "path")?, &allowed),
        "read_file" => tool_read_file(
            arg_path(&call.args, "path")?,
            arg_u64(&call.args, "offset").unwrap_or(0) as usize,
            arg_u64(&call.args, "length").unwrap_or(60 * 1024) as usize,
            &allowed,
        ),
        "write_file" => tool_write_file(
            arg_path(&call.args, "path")?,
            arg_string(&call.args, "content")?,
            &allowed,
        ),
        "file_info" => tool_file_info(arg_path(&call.args, "path")?, &allowed),
        "search_files" => tool_search_files(
            arg_path(&call.args, "root")?,
            arg_string(&call.args, "query")?,
            &allowed,
        ),
        "grep" => tool_grep(
            arg_path(&call.args, "root")?,
            arg_string(&call.args, "query")?,
            &allowed,
        ),
        "analyze_pe_basic" => tool_analyze_pe_basic(arg_path(&call.args, "path")?, &allowed),
        other => Err(format!("未知工具: {other}")),
    }
}

fn arg_string(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("工具参数缺少字符串字段: {key}"))
}

fn arg_path(args: &serde_json::Value, key: &str) -> Result<String, String> {
    arg_string(args, key)
}

fn arg_u64(args: &serde_json::Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|value| value.as_u64())
}

fn checked_existing_path(path: String, allowed: &[PathBuf]) -> Result<PathBuf, String> {
    let path = normalize_path(&path)?;
    if !is_path_allowed(&path, allowed) {
        return Err(format!("未授权路径: {}", path.display()));
    }
    if !path.exists() {
        return Err(format!("路径不存在: {}", path.display()));
    }
    Ok(path)
}

fn checked_target_path(path: String, allowed: &[PathBuf]) -> Result<PathBuf, String> {
    let path = normalize_path(&path)?;
    if !is_path_allowed(&path, allowed) {
        return Err(format!("未授权路径: {}", path.display()));
    }
    Ok(path)
}

fn tool_list_dir(path: String, allowed: &[PathBuf]) -> Result<String, String> {
    let path = checked_existing_path(path, allowed)?;
    if !path.is_dir() {
        return Err(format!("不是目录: {}", path.display()));
    }

    let mut rows = Vec::new();
    for entry in fs::read_dir(&path)
        .map_err(|error| format!("读取目录失败 {}: {error}", path.display()))?
        .take(200)
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let entry_path = entry.path();
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        let marker = if metadata.is_dir() { "dir" } else { "file" };
        rows.push(format!(
            "{}\t{}\t{} bytes",
            marker,
            entry_path.display(),
            metadata.len()
        ));
    }

    Ok(rows.join("\n"))
}

fn tool_read_file(
    path: String,
    offset: usize,
    length: usize,
    allowed: &[PathBuf],
) -> Result<String, String> {
    let path = checked_existing_path(path, allowed)?;
    if !path.is_file() {
        return Err(format!("不是文件: {}", path.display()));
    }

    let bytes = fs::read(&path).map_err(|error| format!("读取文件失败 {}: {error}", path.display()))?;
    let start = offset.min(bytes.len());
    let end = (start + length.min(120 * 1024)).min(bytes.len());
    let slice = &bytes[start..end];
    Ok(format!(
        "path: {}\noffset: {}\nlength: {}\n{}",
        path.display(),
        start,
        slice.len(),
        file_preview(slice)
    ))
}

fn tool_write_file(path: String, content: String, allowed: &[PathBuf]) -> Result<String, String> {
    let path = checked_target_path(path, allowed)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录失败 {}: {error}", parent.display()))?;
    }
    fs::write(&path, content.as_bytes())
        .map_err(|error| format!("写入文件失败 {}: {error}", path.display()))?;
    Ok(format!(
        "已写入 {}，{} 字符",
        path.display(),
        content.chars().count()
    ))
}

fn tool_file_info(path: String, allowed: &[PathBuf]) -> Result<String, String> {
    let path = checked_existing_path(path, allowed)?;
    if path.is_file() {
        summarize_file(&path)
    } else if path.is_dir() {
        tool_list_dir(path.display().to_string(), allowed)
    } else {
        Ok(format!("其他路径类型: {}", path.display()))
    }
}

fn tool_search_files(root: String, query: String, allowed: &[PathBuf]) -> Result<String, String> {
    let root = checked_existing_path(root, allowed)?;
    let root = if root.is_file() {
        root.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("文件没有父目录: {}", root.display()))?
    } else {
        root
    };
    let query = query.to_lowercase();
    let mut hits = Vec::new();
    let mut stack = vec![root];

    while let Some(dir) = stack.pop() {
        if hits.len() >= 200 {
            break;
        }
        for entry in fs::read_dir(&dir)
            .map_err(|error| format!("搜索目录失败 {}: {error}", dir.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_lowercase();
            if should_skip_dir(&path) {
                continue;
            }
            if name.contains(&query) {
                hits.push(path.display().to_string());
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }

    Ok(if hits.is_empty() {
        "没有找到匹配文件。".to_string()
    } else {
        hits.join("\n")
    })
}

fn tool_grep(root: String, query: String, allowed: &[PathBuf]) -> Result<String, String> {
    let root = checked_existing_path(root, allowed)?;
    let mut files = Vec::new();
    if root.is_file() {
        files.push(root);
    } else {
        collect_files(&root, &mut files, 500)?;
    }

    let mut hits = Vec::new();
    for file in files {
        if hits.len() >= 80 {
            break;
        }
        let bytes = fs::read(&file).unwrap_or_default();
        if bytes.len() > 512 * 1024 {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        if !is_probably_text(&text) {
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            if line.contains(&query) {
                hits.push(format!("{}:{}: {}", file.display(), index + 1, line));
                if hits.len() >= 80 {
                    break;
                }
            }
        }
    }

    Ok(if hits.is_empty() {
        "没有找到匹配文本。".to_string()
    } else {
        hits.join("\n")
    })
}

fn tool_analyze_pe_basic(path: String, allowed: &[PathBuf]) -> Result<String, String> {
    let path = checked_existing_path(path, allowed)?;
    if !path.is_file() {
        return Err(format!("不是文件: {}", path.display()));
    }
    let bytes = fs::read(&path).map_err(|error| format!("读取文件失败 {}: {error}", path.display()))?;
    let mut report = vec![summarize_file(&path)?];

    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        report.push("PE 检查: 不是 MZ/PE 文件。".to_string());
        return Ok(report.join("\n\n"));
    }

    let pe_offset = read_u32(&bytes, 0x3c).unwrap_or(0) as usize;
    if bytes.len() < pe_offset + 24 || &bytes[pe_offset..pe_offset + 4] != b"PE\0\0" {
        report.push("PE 检查: MZ 存在，但 PE 头无效。".to_string());
        return Ok(report.join("\n\n"));
    }

    let machine = read_u16(&bytes, pe_offset + 4).unwrap_or(0);
    let sections = read_u16(&bytes, pe_offset + 6).unwrap_or(0);
    let timestamp = read_u32(&bytes, pe_offset + 8).unwrap_or(0);
    let optional_size = read_u16(&bytes, pe_offset + 20).unwrap_or(0) as usize;
    let optional_offset = pe_offset + 24;
    let optional_magic = read_u16(&bytes, optional_offset).unwrap_or(0);
    let entry_point = read_u32(&bytes, optional_offset + 16).unwrap_or(0);
    let image_base = if optional_magic == 0x20b {
        read_u64(&bytes, optional_offset + 24).unwrap_or(0)
    } else {
        read_u32(&bytes, optional_offset + 28).unwrap_or(0) as u64
    };

    report.push(format!(
        "PE 基础信息:\nMachine: 0x{machine:04x} ({})\nSections: {sections}\nTimeDateStamp: {timestamp}\nOptionalMagic: 0x{optional_magic:04x}\nEntryPointRVA: 0x{entry_point:08x}\nImageBase: 0x{image_base:x}",
        machine_name(machine)
    ));

    let section_offset = optional_offset + optional_size;
    let mut section_rows = Vec::new();
    for index in 0..sections as usize {
        let offset = section_offset + index * 40;
        if bytes.len() < offset + 40 {
            break;
        }
        let name_bytes = &bytes[offset..offset + 8];
        let name = String::from_utf8_lossy(
            &name_bytes
                .iter()
                .copied()
                .take_while(|byte| *byte != 0)
                .collect::<Vec<_>>(),
        )
        .to_string();
        let virtual_size = read_u32(&bytes, offset + 8).unwrap_or(0);
        let virtual_address = read_u32(&bytes, offset + 12).unwrap_or(0);
        let raw_size = read_u32(&bytes, offset + 16).unwrap_or(0);
        let characteristics = read_u32(&bytes, offset + 36).unwrap_or(0);
        section_rows.push(format!(
            "{}\tRVA=0x{:08x}\tVirtualSize={}\tRawSize={}\tCharacteristics=0x{:08x}",
            name, virtual_address, virtual_size, raw_size, characteristics
        ));
    }
    if !section_rows.is_empty() {
        report.push(format!("PE 节区:\n{}", section_rows.join("\n")));
    }

    Ok(report.join("\n\n"))
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>, limit: usize) -> Result<(), String> {
    if files.len() >= limit || should_skip_dir(dir) {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|error| format!("读取目录失败 {}: {error}", dir.display()))? {
        if files.len() >= limit {
            break;
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files, limit)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| matches!(name, ".git" | "node_modules" | "target" | "dist"))
        .unwrap_or(false)
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

fn machine_name(machine: u16) -> &'static str {
    match machine {
        0x014c => "x86",
        0x8664 => "x64",
        0xaa64 => "ARM64",
        0x01c4 => "ARM",
        _ => "unknown",
    }
}

fn apply_ai2_file_writes(ai2_result: &str, allowed_paths: &[String]) -> Result<Vec<String>, String> {
    let allowed = normalize_allowed_paths(allowed_paths)?;
    if allowed.is_empty() {
        return Ok(Vec::new());
    }

    let writes = parse_write_blocks(ai2_result);
    let mut summaries = Vec::new();

    for write in writes {
        let target = normalize_path(&write.path)?;
        if !is_path_allowed(&target, &allowed) {
            return Err(format!("AI2 尝试写入未授权路径: {}", target.display()));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建目录失败 {}: {error}", parent.display()))?;
        }
        fs::write(&target, write.content.as_bytes())
            .map_err(|error| format!("写入文件失败 {}: {error}", target.display()))?;
        summaries.push(format!(
            "已写入 {} ({} 字符)",
            target.display(),
            write.content.chars().count()
        ));
    }

    Ok(summaries)
}

fn append_write_summary(mut ai2_result: String, summaries: Vec<String>) -> String {
    if summaries.is_empty() {
        return ai2_result;
    }
    ai2_result.push_str("\n\n[本地文件写入结果]\n");
    ai2_result.push_str(&summaries.join("\n"));
    ai2_result
}

struct FileWriteBlock {
    path: String,
    content: String,
}

fn parse_write_blocks(input: &str) -> Vec<FileWriteBlock> {
    let mut blocks = Vec::new();
    let mut rest = input;
    let start_marker = "<write_file path=\"";
    let end_marker = "</write_file>";

    while let Some(start) = rest.find(start_marker) {
        let after_start = &rest[start + start_marker.len()..];
        let Some(path_end) = after_start.find("\">") else {
            break;
        };
        let path = after_start[..path_end].trim().to_string();
        let after_path = &after_start[path_end + 2..];
        let Some(content_end) = after_path.find(end_marker) else {
            break;
        };
        let content = after_path[..content_end]
            .trim_start_matches('\n')
            .trim_end_matches('\n')
            .to_string();
        blocks.push(FileWriteBlock { path, content });
        rest = &after_path[content_end + end_marker.len()..];
    }

    blocks
}

fn normalize_allowed_paths(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(normalize_path)
        .collect()
}

fn normalize_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if path.exists() {
        path.canonicalize()
            .map_err(|error| format!("解析路径失败 {}: {error}", path.display()))
    } else if let Some(parent) = path.parent() {
        let parent = parent
            .canonicalize()
            .map_err(|error| format!("解析父目录失败 {}: {error}", parent.display()))?;
        let Some(file_name) = path.file_name() else {
            return Err(format!("路径无效: {}", path.display()));
        };
        Ok(parent.join(file_name))
    } else {
        Err(format!("路径无效: {}", path.display()))
    }
}

fn is_path_allowed(target: &Path, allowed_paths: &[PathBuf]) -> bool {
    allowed_paths.iter().any(|allowed| {
        if allowed.is_file() {
            target == allowed
        } else {
            target.starts_with(allowed)
        }
    })
}

fn detect_success(
    mode: &SuccessMode,
    keywords: &[String],
    ai2_result: &str,
    ai1_review: &str,
) -> bool {
    match mode {
        SuccessMode::Manual => false,
        SuccessMode::Ai1Judgement => ai1_review.contains('成')
            && ai1_review.contains('功')
            && !ai1_review.contains("继续"),
        SuccessMode::Keyword => keywords
            .iter()
            .map(|keyword| keyword.trim())
            .filter(|keyword| !keyword.is_empty())
            .any(|keyword| ai2_result.contains(keyword) || ai1_review.contains(keyword)),
    }
}

fn should_run_round(round_index: u32, max_rounds: u32) -> bool {
    max_rounds == 0 || round_index <= max_rounds
}

fn chat_endpoint(base_url: &str) -> String {
    let clean = base_url.trim().trim_end_matches('/');
    if clean.ends_with("/chat/completions") {
        clean.to_string()
    } else {
        format!("{clean}/chat/completions")
    }
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn default_settings() -> Settings {
    Settings {
        providers: vec![
            ProviderConfig {
                id: "ai1-default".to_string(),
                name: "AI1 审核模型".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                api_key: String::new(),
                model: "gpt-4.1-mini".to_string(),
                temperature: 0.2,
            },
            ProviderConfig {
                id: "ai2-default".to_string(),
                name: "AI2 执行模型".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                api_key: String::new(),
                model: "gpt-4.1-mini".to_string(),
                temperature: 0.4,
            },
        ],
        role_config: RoleConfig {
            ai1_provider_id: "ai1-default".to_string(),
            ai2_provider_id: "ai2-default".to_string(),
            ai1_system_prompt: "你是 AI1，负责分析用户需求、拆解执行方案，并审核 AI2 的结果是否达到目标。".to_string(),
            ai2_system_prompt: "你是 AI2，负责严格执行 AI1 给出的方案，只返回执行结果。".to_string(),
        },
        templates: vec![PromptTemplate {
            id: "default".to_string(),
            name: "默认执行模板".to_string(),
            prefix: "请根据下面的方案执行。".to_string(),
            suffix: "你的目标是尽量完整、准确地完成用户需求。".to_string(),
        }],
        active_template_id: "default".to_string(),
        ai2_local_paths: Vec::new(),
        display_mode: DisplayMode::Dual,
        success_mode: SuccessMode::Ai1Judgement,
        success_keywords: vec!["成功".to_string()],
        theme: ThemeMode::Dark,
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            list_tasks,
            get_task,
            start_task,
            pause_task,
            resume_task,
            stop_task,
            delete_task,
            test_provider_connection
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_prompt_uses_prefix_content_suffix() {
        let template = PromptTemplate {
            id: "t".into(),
            name: "T".into(),
            prefix: "前缀".into(),
            suffix: "后缀".into(),
        };

        assert_eq!(
            compose_ai2_prompt(&template, "正文", "文件"),
            "前缀\n\n正文\n\n文件\n\n后缀"
        );
    }

    #[test]
    fn max_round_zero_means_unlimited() {
        assert!(should_run_round(1, 0));
        assert!(should_run_round(100, 0));
        assert!(should_run_round(2, 2));
        assert!(!should_run_round(3, 2));
    }

    #[test]
    fn success_modes_are_distinct() {
        assert!(detect_success(
            &SuccessMode::Ai1Judgement,
            &[],
            "",
            "审核结论：成功"
        ));
        assert!(!detect_success(
            &SuccessMode::Ai1Judgement,
            &[],
            "",
            "还没有成功，需要继续"
        ));
        assert!(detect_success(
            &SuccessMode::Keyword,
            &["DONE".to_string()],
            "result: DONE",
            ""
        ));
        assert!(!detect_success(
            &SuccessMode::Manual,
            &["DONE".to_string()],
            "result: DONE",
            "成功"
        ));
    }

    #[test]
    fn storage_round_trip_uses_plain_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(DATA_FILE);
        let data = StoredData::default();

        write_data_to_path(&path, &data).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("apiKey"));

        let loaded = read_data_from_path(&path).unwrap();
        assert_eq!(loaded.settings.templates[0].id, "default");
    }

    #[test]
    fn parses_ai2_write_blocks() {
        let blocks = parse_write_blocks(
            "done\n<write_file path=\"C:\\\\tmp\\\\a.txt\">\nhello\n</write_file>",
        );

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].path, "C:\\\\tmp\\\\a.txt");
        assert_eq!(blocks[0].content, "hello");
    }

    #[test]
    fn parses_ai2_tool_calls() {
        let calls = parse_tool_calls(
            "<tool_call name=\"read_file\">\n{\"path\":\"D:\\\\p\\\\a.txt\"}\n</tool_call>",
        );

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].args["path"], "D:\\\\p\\\\a.txt");
    }
}
