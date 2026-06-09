import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  CheckCircle2,
  Clock3,
  Columns3,
  ListChecks,
  Moon,
  Pause,
  Play,
  Plus,
  Save,
  Settings as SettingsIcon,
  Square,
  Sun,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

type DisplayMode = "dual" | "timeline" | "focus";
type SuccessMode = "ai1Judgement" | "manual" | "keyword";
type ThemeMode = "dark" | "light";
type TaskStatus =
  | "running"
  | "paused"
  | "stopped"
  | "succeeded"
  | "failed"
  | "limitReached";
type RoundStatus = "completed" | "succeeded" | "failed";

type ProviderConfig = {
  id: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  model: string;
  temperature: number;
};

type RoleConfig = {
  ai1ProviderId: string;
  ai2ProviderId: string;
  ai1SystemPrompt: string;
  ai2SystemPrompt: string;
};

type PromptTemplate = {
  id: string;
  name: string;
  prefix: string;
  suffix: string;
};

type Settings = {
  providers: ProviderConfig[];
  roleConfig: RoleConfig;
  templates: PromptTemplate[];
  activeTemplateId: string;
  ai2LocalPaths: string[];
  displayMode: DisplayMode;
  successMode: SuccessMode;
  successKeywords: string[];
  theme: ThemeMode;
};

type Round = {
  index: number;
  ai1Plan: string;
  ai2Prompt: string;
  ai2Result: string;
  ai1Review: string;
  status: RoundStatus;
  durationMs: number;
};

type TaskRecord = {
  id: string;
  userGoal: string;
  maxRounds: number;
  status: TaskStatus;
  successMode: SuccessMode;
  displayMode: DisplayMode;
  rounds: Round[];
  finalResult: string;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
};

type StartTaskRequest = {
  userGoal: string;
  maxRounds: number;
  settings: Settings;
};

const defaultSettings: Settings = {
  providers: [
    {
      id: "ai1-default",
      name: "AI1 审核模型",
      baseUrl: "https://api.openai.com/v1",
      apiKey: "",
      model: "gpt-4.1-mini",
      temperature: 0.2,
    },
    {
      id: "ai2-default",
      name: "AI2 执行模型",
      baseUrl: "https://api.openai.com/v1",
      apiKey: "",
      model: "gpt-4.1-mini",
      temperature: 0.4,
    },
  ],
  roleConfig: {
    ai1ProviderId: "ai1-default",
    ai2ProviderId: "ai2-default",
    ai1SystemPrompt:
      "你是 AI1，负责分析用户需求、拆解执行方案，并审核 AI2 的结果是否达到目标。",
    ai2SystemPrompt: "你是 AI2，负责严格执行 AI1 给出的方案，只返回执行结果。",
  },
  templates: [
    {
      id: "default",
      name: "默认执行模板",
      prefix: "请根据下面的方案执行。",
      suffix: "你的目标是尽量完整、准确地完成用户需求。",
    },
  ],
  activeTemplateId: "default",
  ai2LocalPaths: [],
  displayMode: "dual",
  successMode: "ai1Judgement",
  successKeywords: ["成功"],
  theme: "dark",
};

const cloneSettings = (settings: Settings = defaultSettings) =>
  JSON.parse(JSON.stringify(settings)) as Settings;

const statusText: Record<TaskStatus, string> = {
  running: "运行中",
  paused: "已暂停",
  stopped: "已停止",
  succeeded: "已成功",
  failed: "失败",
  limitReached: "达到轮数",
};

const successModeText: Record<SuccessMode, string> = {
  ai1Judgement: "AI1 判定",
  manual: "用户手动",
  keyword: "关键词",
};

const displayModeText: Record<DisplayMode, string> = {
  dual: "双栏",
  timeline: "时间线",
  focus: "当前轮",
};

function App() {
  const [settings, setSettings] = useState<Settings>(cloneSettings());
  const [settingsDraft, setSettingsDraft] = useState<Settings>(cloneSettings());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [goal, setGoal] = useState("");
  const [maxRounds, setMaxRounds] = useState(3);
  const [tasks, setTasks] = useState<TaskRecord[]>([]);
  const [currentTaskId, setCurrentTaskId] = useState<string>("");
  const [error, setError] = useState("");
  const [isStarting, setIsStarting] = useState(false);
  const [providerTests, setProviderTests] = useState<
    Record<string, { status: "idle" | "testing" | "success" | "error"; message: string }>
  >({});

  const currentTask = useMemo(
    () => tasks.find((task) => task.id === currentTaskId) ?? tasks[0],
    [currentTaskId, tasks],
  );

  const isLiveTask =
    currentTask?.status === "running" || currentTask?.status === "paused";

  useEffect(() => {
    void bootstrap();
    if (!isTauriRuntime()) return;

    let unlisten: null | (() => void) = null;
    void listen<TaskRecord>("task-updated", (event) => {
      setTasks((items) => upsertTask(items, event.payload));
      setCurrentTaskId(event.payload.id);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = settings.theme;
  }, [settings.theme]);

  async function bootstrap() {
    if (!isTauriRuntime()) {
      setSettings(cloneSettings());
      setSettingsDraft(cloneSettings());
      return;
    }

    try {
      const [loadedSettings, loadedTasks] = await Promise.all([
        invoke<Settings>("load_settings"),
        invoke<TaskRecord[]>("list_tasks"),
      ]);
      setSettings(loadedSettings);
      setSettingsDraft(cloneSettings(loadedSettings));
      setTasks(sortTasks(loadedTasks));
      setCurrentTaskId(loadedTasks[0]?.id ?? "");
    } catch {
      setSettings(cloneSettings());
      setSettingsDraft(cloneSettings());
    }
  }

  async function startTask() {
    const trimmedGoal = goal.trim();
    if (!trimmedGoal) {
      setError("请先输入用户需求。");
      return;
    }

    setError("");
    setIsStarting(true);
    try {
      if (!isTauriRuntime()) {
        throw new Error("当前是浏览器预览模式，请在 Tauri 桌面应用中运行任务。");
      }
      const request: StartTaskRequest = {
        userGoal: trimmedGoal,
        maxRounds: Math.max(0, Number(maxRounds) || 0),
        settings,
      };
      const task = await invoke<TaskRecord>("start_task", { request });
      setTasks((items) => upsertTask(items, task));
      setCurrentTaskId(task.id);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setIsStarting(false);
    }
  }

  async function controlTask(command: "pause_task" | "resume_task" | "stop_task") {
    if (!currentTask) return;
    setError("");
    try {
      if (!isTauriRuntime()) {
        throw new Error("当前是浏览器预览模式，请在 Tauri 桌面应用中控制任务。");
      }
      await invoke(command, { taskId: currentTask.id });
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function removeTask(taskId: string) {
    setError("");
    try {
      if (!isTauriRuntime()) {
        setTasks((items) => items.filter((task) => task.id !== taskId));
        return;
      }
      await invoke("delete_task", { taskId });
      setTasks((items) => items.filter((task) => task.id !== taskId));
      if (currentTaskId === taskId) setCurrentTaskId("");
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function saveSettings(next: Settings) {
    setError("");
    try {
      if (!isTauriRuntime()) {
        setSettings(next);
        setSettingsDraft(cloneSettings(next));
        setSettingsOpen(false);
        return;
      }
      const saved = await invoke<Settings>("save_settings", { settings: next });
      setSettings(saved);
      setSettingsDraft(cloneSettings(saved));
      setSettingsOpen(false);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function testProvider(provider: ProviderConfig, systemPrompt: string) {
    setProviderTests((items) => ({
      ...items,
      [provider.id]: { status: "testing", message: "正在测试连接..." },
    }));

    try {
      if (!isTauriRuntime()) {
        throw new Error("当前是浏览器预览模式，请在 Tauri 桌面应用中测试连接。");
      }

      const message = await invoke<string>("test_provider_connection", {
        request: { provider, systemPrompt },
      });
      setProviderTests((items) => ({
        ...items,
        [provider.id]: { status: "success", message },
      }));
    } catch (reason) {
      setProviderTests((items) => ({
        ...items,
        [provider.id]: { status: "error", message: String(reason) },
      }));
    }
  }

  function openSettings() {
    setSettingsDraft(cloneSettings(settings));
    setSettingsOpen(true);
  }

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div>
            <p className="eyebrow">Chat to Chat</p>
            <h1>双 AI 多轮执行器</h1>
          </div>
          <button className="icon-button" onClick={openSettings} title="设置">
            <SettingsIcon size={18} />
          </button>
        </div>

        <div className="task-list">
          {tasks.length === 0 ? (
            <p className="empty">还没有任务历史。</p>
          ) : (
            tasks.map((task) => (
              <button
                className={`task-item ${task.id === currentTask?.id ? "active" : ""}`}
                key={task.id}
                onClick={() => setCurrentTaskId(task.id)}
              >
                <span className={`status-dot ${task.status}`} />
                <span>
                  <strong>{task.userGoal}</strong>
                  <small>
                    {statusText[task.status]} · {task.rounds.length}
                    {task.maxRounds === 0 ? "/∞" : `/${task.maxRounds}`} 轮
                  </small>
                </span>
              </button>
            ))
          )}
        </div>
      </aside>

      <section className="workspace">
        <header className="toolbar">
          <div className="goal-box">
            <label htmlFor="goal">用户需求</label>
            <textarea
              id="goal"
              value={goal}
              placeholder="输入用户给 AI1 的需求..."
              onChange={(event) => setGoal(event.target.value)}
            />
          </div>

          <div className="run-controls">
            <label>
              轮数
              <input
                type="number"
                min={0}
                value={maxRounds}
                onChange={(event) => setMaxRounds(Number(event.target.value))}
              />
              <small>0 表示直到成功</small>
            </label>
            <button className="primary-button" onClick={startTask} disabled={isStarting}>
              <Play size={17} />
              {isStarting ? "启动中" : "开始"}
            </button>
            <button
              className="ghost-button"
              onClick={() =>
                controlTask(currentTask?.status === "paused" ? "resume_task" : "pause_task")
              }
              disabled={!isLiveTask}
            >
              {currentTask?.status === "paused" ? <Play size={17} /> : <Pause size={17} />}
              {currentTask?.status === "paused" ? "继续" : "暂停"}
            </button>
            <button
              className="ghost-button danger"
              onClick={() => controlTask("stop_task")}
              disabled={!isLiveTask}
            >
              <Square size={16} />
              停止
            </button>
          </div>
        </header>

        {error ? <div className="error-bar">{error}</div> : null}

        <section className="meta-row">
          <InfoBadge icon={<Columns3 size={15} />} label={displayModeText[settings.displayMode]} />
          <InfoBadge
            icon={<ListChecks size={15} />}
            label={successModeText[settings.successMode]}
          />
          <InfoBadge
            icon={<Clock3 size={15} />}
            label={currentTask ? statusText[currentTask.status] : "待开始"}
          />
          <InfoBadge
            icon={settings.theme === "dark" ? <Moon size={15} /> : <Sun size={15} />}
            label={settings.theme === "dark" ? "深色" : "浅色"}
          />
        </section>

        <TaskView task={currentTask} mode={settings.displayMode} onDelete={removeTask} />
      </section>

      {settingsOpen ? (
        <SettingsModal
          draft={settingsDraft}
          setDraft={setSettingsDraft}
          onClose={() => setSettingsOpen(false)}
          onSave={saveSettings}
          providerTests={providerTests}
          onTestProvider={testProvider}
        />
      ) : null}
    </main>
  );
}

function TaskView({
  task,
  mode,
  onDelete,
}: {
  task?: TaskRecord;
  mode: DisplayMode;
  onDelete: (taskId: string) => void;
}) {
  if (!task) {
    return (
      <section className="blank-state">
        <CheckCircle2 size={28} />
        <h2>准备一个目标，然后开始第一轮。</h2>
      </section>
    );
  }

  const latest = task.rounds[task.rounds.length - 1];

  return (
    <section className="task-view">
      <div className="task-heading">
        <div>
          <p className="eyebrow">当前任务</p>
          <h2>{task.userGoal}</h2>
          <span>
            {statusText[task.status]} · {task.rounds.length}
            {task.maxRounds === 0 ? "/∞" : `/${task.maxRounds}`} 轮
          </span>
        </div>
        <button className="icon-button danger" onClick={() => onDelete(task.id)} title="删除任务">
          <Trash2 size={17} />
        </button>
      </div>

      {task.error ? <div className="error-bar">{task.error}</div> : null}

      {mode === "dual" ? <DualRounds rounds={task.rounds} /> : null}
      {mode === "timeline" ? <TimelineRounds rounds={task.rounds} /> : null}
      {mode === "focus" ? <FocusRound round={latest} /> : null}

      {task.finalResult ? (
        <article className="final-result">
          <h3>最终结果</h3>
          <pre>{task.finalResult}</pre>
        </article>
      ) : null}
    </section>
  );
}

function DualRounds({ rounds }: { rounds: Round[] }) {
  if (rounds.length === 0) return <p className="empty">等待 AI1 生成第一轮方案。</p>;

  return (
    <div className="dual-grid">
      <div>
        <h3>AI1 分析 / 审核</h3>
        {rounds.map((round) => (
          <RoundCard key={`ai1-${round.index}`} title={`第 ${round.index} 轮`} tone="ai1">
            <pre>{round.ai1Plan}</pre>
            <pre>{round.ai1Review}</pre>
          </RoundCard>
        ))}
      </div>
      <div>
        <h3>AI2 执行结果</h3>
        {rounds.map((round) => (
          <RoundCard key={`ai2-${round.index}`} title={`第 ${round.index} 轮`} tone="ai2">
            <pre>{round.ai2Result}</pre>
          </RoundCard>
        ))}
      </div>
    </div>
  );
}

function TimelineRounds({ rounds }: { rounds: Round[] }) {
  if (rounds.length === 0) return <p className="empty">还没有轮次。</p>;

  return (
    <div className="timeline">
      {rounds.map((round) => (
        <RoundCard key={round.index} title={`第 ${round.index} 轮 · ${round.durationMs}ms`}>
          <h4>AI1 方案</h4>
          <pre>{round.ai1Plan}</pre>
          <h4>AI2 执行</h4>
          <pre>{round.ai2Result}</pre>
          <h4>AI1 审核</h4>
          <pre>{round.ai1Review}</pre>
        </RoundCard>
      ))}
    </div>
  );
}

function FocusRound({ round }: { round?: Round }) {
  if (!round) return <p className="empty">当前还没有完成的轮次。</p>;

  return (
    <div className="focus-round">
      <RoundCard title={`当前轮：第 ${round.index} 轮`}>
        <h4>发送给 AI2 的内容</h4>
        <pre>{round.ai2Prompt}</pre>
        <h4>AI2 返回</h4>
        <pre>{round.ai2Result}</pre>
        <h4>AI1 审核</h4>
        <pre>{round.ai1Review}</pre>
      </RoundCard>
    </div>
  );
}

function RoundCard({
  title,
  tone,
  children,
}: {
  title: string;
  tone?: "ai1" | "ai2";
  children: React.ReactNode;
}) {
  return (
    <article className={`round-card ${tone ?? ""}`}>
      <div className="round-title">{title}</div>
      {children}
    </article>
  );
}

function SettingsModal({
  draft,
  setDraft,
  onClose,
  onSave,
  providerTests,
  onTestProvider,
}: {
  draft: Settings;
  setDraft: (settings: Settings) => void;
  onClose: () => void;
  onSave: (settings: Settings) => Promise<void>;
  providerTests: Record<
    string,
    { status: "idle" | "testing" | "success" | "error"; message: string }
  >;
  onTestProvider: (provider: ProviderConfig, systemPrompt: string) => Promise<void>;
}) {
  const ai1Provider = draft.providers.find(
    (provider) => provider.id === draft.roleConfig.ai1ProviderId,
  );
  const ai2Provider = draft.providers.find(
    (provider) => provider.id === draft.roleConfig.ai2ProviderId,
  );
  const activeTemplate =
    draft.templates.find((template) => template.id === draft.activeTemplateId) ??
    draft.templates[0];

  function updateProvider(id: string, patch: Partial<ProviderConfig>) {
    setDraft({
      ...draft,
      providers: draft.providers.map((provider) =>
        provider.id === id ? { ...provider, ...patch } : provider,
      ),
    });
  }

  function updateRoleConfig(patch: Partial<RoleConfig>) {
    setDraft({ ...draft, roleConfig: { ...draft.roleConfig, ...patch } });
  }

  function updateActiveTemplate(patch: Partial<PromptTemplate>) {
    setDraft({
      ...draft,
      templates: draft.templates.map((template) =>
        template.id === activeTemplate.id ? { ...template, ...patch } : template,
      ),
    });
  }

  function addTemplate() {
    const id = crypto.randomUUID();
    setDraft({
      ...draft,
      activeTemplateId: id,
      templates: [
        ...draft.templates,
        {
          id,
          name: `模板 ${draft.templates.length + 1}`,
          prefix: "",
          suffix: "",
        },
      ],
    });
  }

  function deleteTemplate() {
    if (draft.templates.length <= 1) return;
    const nextTemplates = draft.templates.filter((template) => template.id !== activeTemplate.id);
    setDraft({
      ...draft,
      templates: nextTemplates,
      activeTemplateId: nextTemplates[0].id,
    });
  }

  async function chooseLocalPaths(kind: "file" | "directory") {
    if (!isTauriRuntime()) return;
    const selected = await open({
      multiple: true,
      directory: kind === "directory",
      title: kind === "directory" ? "选择 AI2 可读写目录" : "选择 AI2 可读写文件",
    });
    if (!selected) return;

    const selectedPaths = Array.isArray(selected) ? selected : [selected];
    setDraft({
      ...draft,
      ai2LocalPaths: Array.from(new Set([...draft.ai2LocalPaths, ...selectedPaths])),
    });
  }

  function removeLocalPath(path: string) {
    setDraft({
      ...draft,
      ai2LocalPaths: draft.ai2LocalPaths.filter((item) => item !== path),
    });
  }

  return (
    <div className="modal-backdrop">
      <section className="settings-modal">
        <header>
          <div>
            <p className="eyebrow">Settings</p>
            <h2>运行设置</h2>
          </div>
          <div className="modal-actions">
            <button className="ghost-button" onClick={onClose}>
              取消
            </button>
            <button className="primary-button" onClick={() => onSave(draft)}>
              <Save size={17} />
              保存
            </button>
          </div>
        </header>

        <div className="settings-grid">
          {ai1Provider ? (
            <ProviderPanel
              title="AI1 分析 / 审核"
              provider={ai1Provider}
              systemPrompt={draft.roleConfig.ai1SystemPrompt}
              testState={providerTests[ai1Provider.id]}
              onProviderChange={(patch) => updateProvider(ai1Provider.id, patch)}
              onSystemPromptChange={(value) => updateRoleConfig({ ai1SystemPrompt: value })}
              onTest={() => onTestProvider(ai1Provider, draft.roleConfig.ai1SystemPrompt)}
            />
          ) : null}
          {ai2Provider ? (
            <ProviderPanel
              title="AI2 执行"
              provider={ai2Provider}
              systemPrompt={draft.roleConfig.ai2SystemPrompt}
              testState={providerTests[ai2Provider.id]}
              onProviderChange={(patch) => updateProvider(ai2Provider.id, patch)}
              onSystemPromptChange={(value) => updateRoleConfig({ ai2SystemPrompt: value })}
              onTest={() => onTestProvider(ai2Provider, draft.roleConfig.ai2SystemPrompt)}
            />
          ) : null}

          <section className="settings-section">
            <h3>流程偏好</h3>
            <label>
              展示模式
              <select
                value={draft.displayMode}
                onChange={(event) =>
                  setDraft({ ...draft, displayMode: event.target.value as DisplayMode })
                }
              >
                <option value="dual">双栏对话流</option>
                <option value="timeline">时间线列表</option>
                <option value="focus">简洁当前轮</option>
              </select>
            </label>
            <label>
              成功判断
              <select
                value={draft.successMode}
                onChange={(event) =>
                  setDraft({ ...draft, successMode: event.target.value as SuccessMode })
                }
              >
                <option value="ai1Judgement">AI1 明确判定</option>
                <option value="manual">用户手动停止</option>
                <option value="keyword">关键词匹配</option>
              </select>
            </label>
            <label>
              成功关键词
              <input
                value={draft.successKeywords.join(", ")}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    successKeywords: event.target.value
                      .split(",")
                      .map((item) => item.trim())
                      .filter(Boolean),
                  })
                }
              />
            </label>
            <div className="path-picker">
              <div className="section-title compact">
                <label>AI2 本地读写路径</label>
                <div className="path-actions">
                  <button
                    className="ghost-button"
                    onClick={() => chooseLocalPaths("file")}
                    disabled={!isTauriRuntime()}
                  >
                    选择文件
                  </button>
                  <button
                    className="ghost-button"
                    onClick={() => chooseLocalPaths("directory")}
                    disabled={!isTauriRuntime()}
                  >
                    选择目录
                  </button>
                </div>
              </div>
              <div className="path-list">
                {draft.ai2LocalPaths.length === 0 ? (
                  <p className="empty">还没有授权路径。</p>
                ) : (
                  draft.ai2LocalPaths.map((path) => (
                    <div className="path-item" key={path}>
                      <span>{path}</span>
                      <button
                        className="icon-button danger"
                        onClick={() => removeLocalPath(path)}
                        title="移除路径"
                      >
                        <Trash2 size={15} />
                      </button>
                    </div>
                  ))
                )}
              </div>
            </div>
            <p className="plain-warning">
              AI2 可读取这些路径；若输出 &lt;write_file path=&quot;...&quot;&gt;内容&lt;/write_file&gt;，后端会写入授权路径内的文件。路径只能通过系统选择框添加。
            </p>
            <div className="segmented">
              <button
                className={draft.theme === "dark" ? "selected" : ""}
                onClick={() => setDraft({ ...draft, theme: "dark" })}
              >
                <Moon size={16} />
                深色
              </button>
              <button
                className={draft.theme === "light" ? "selected" : ""}
                onClick={() => setDraft({ ...draft, theme: "light" })}
              >
                <Sun size={16} />
                浅色
              </button>
            </div>
          </section>

          <section className="settings-section template-section">
            <div className="section-title">
              <h3>前后缀模板</h3>
              <div>
                <button className="icon-button" onClick={addTemplate} title="新增模板">
                  <Plus size={17} />
                </button>
                <button
                  className="icon-button danger"
                  onClick={deleteTemplate}
                  disabled={draft.templates.length <= 1}
                  title="删除模板"
                >
                  <Trash2 size={17} />
                </button>
              </div>
            </div>
            <label>
              当前模板
              <select
                value={draft.activeTemplateId}
                onChange={(event) => setDraft({ ...draft, activeTemplateId: event.target.value })}
              >
                {draft.templates.map((template) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              模板名
              <input
                value={activeTemplate.name}
                onChange={(event) => updateActiveTemplate({ name: event.target.value })}
              />
            </label>
            <label>
              固定前缀
              <textarea
                value={activeTemplate.prefix}
                onChange={(event) => updateActiveTemplate({ prefix: event.target.value })}
              />
            </label>
            <label>
              固定后缀
              <textarea
                value={activeTemplate.suffix}
                onChange={(event) => updateActiveTemplate({ suffix: event.target.value })}
              />
            </label>
          </section>
        </div>
      </section>
    </div>
  );
}

function ProviderPanel({
  title,
  provider,
  systemPrompt,
  testState,
  onProviderChange,
  onSystemPromptChange,
  onTest,
}: {
  title: string;
  provider: ProviderConfig;
  systemPrompt: string;
  testState?: { status: "idle" | "testing" | "success" | "error"; message: string };
  onProviderChange: (patch: Partial<ProviderConfig>) => void;
  onSystemPromptChange: (value: string) => void;
  onTest: () => Promise<void>;
}) {
  const canTest =
    provider.baseUrl.trim().length > 0 &&
    provider.apiKey.trim().length > 0 &&
    provider.model.trim().length > 0 &&
    testState?.status !== "testing";

  return (
    <section className="settings-section">
      <div className="section-title compact">
        <h3>{title}</h3>
        <button className="ghost-button test-button" onClick={onTest} disabled={!canTest}>
          <Play size={15} />
          {testState?.status === "testing" ? "测试中" : "测试连接"}
        </button>
      </div>
      <label>
        名称
        <input
          value={provider.name}
          onChange={(event) => onProviderChange({ name: event.target.value })}
        />
      </label>
      <label>
        Base URL
        <input
          value={provider.baseUrl}
          placeholder="https://api.openai.com/v1"
          onChange={(event) => onProviderChange({ baseUrl: event.target.value })}
        />
      </label>
      <label>
        API Key
        <input
          type="password"
          value={provider.apiKey}
          onChange={(event) => onProviderChange({ apiKey: event.target.value })}
        />
      </label>
      <label>
        Model
        <input
          value={provider.model}
          onChange={(event) => onProviderChange({ model: event.target.value })}
        />
      </label>
      <label>
        Temperature
        <input
          type="number"
          min={0}
          max={2}
          step={0.1}
          value={provider.temperature}
          onChange={(event) => onProviderChange({ temperature: Number(event.target.value) })}
        />
      </label>
      <label>
        System Prompt
        <textarea
          value={systemPrompt}
          onChange={(event) => onSystemPromptChange(event.target.value)}
        />
      </label>
      {testState?.message ? (
        <p className={`test-result ${testState.status}`}>{testState.message}</p>
      ) : null}
      <p className="plain-warning">配置会以明文 JSON 保存到本地应用数据目录。</p>
    </section>
  );
}

function InfoBadge({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <span className="info-badge">
      {icon}
      {label}
    </span>
  );
}

function sortTasks(tasks: TaskRecord[]) {
  return [...tasks].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

function upsertTask(tasks: TaskRecord[], task: TaskRecord) {
  const next = tasks.some((item) => item.id === task.id)
    ? tasks.map((item) => (item.id === task.id ? task : item))
    : [task, ...tasks];
  return sortTasks(next);
}

function isTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export default App;
