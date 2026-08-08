import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { confirm as dialogConfirm, message as dialogMessage } from "@tauri-apps/plugin-dialog";
import Sortable from "sortablejs";
import {
  cellValuesForOrder,
  moveInArray,
  resizeColumnBoundary
} from "./table-model.js";
import { columnDragOptions } from "./table-drag.js";
import {
  newRule,
  parseKeywords,
  ruleSummary,
  validateRule
} from "./rules-model.js";

const PAGES = [
  { id: "push", label: "推送" },
  { id: "rules", label: "规则" },
  { id: "settings", label: "设置" },
  { id: "about", label: "关于" }
];

let currentPage = "push";
let config = null;
let uiState = null;
let pushTable = null;
let rules = [];
let editingId = null;

const COLUMNS = [
  { id: "time", title: "时间" },
  { id: "title", title: "标题" },
  { id: "message", title: "内容" }
];

const MIN_COLUMN_WIDTHS = { time: 120, title: 80, message: 160 };

function el(id) { return document.getElementById(id); }

async function confirmBox(message, title) {
  try {
    return await dialogConfirm(message, { title, kind: "warning" });
  } catch {
    return false;
  }
}

async function alertBox(message, title) {
  try {
    await dialogMessage(message, { title });
  } catch {
    // 平台不支持原生消息框时静默
  }
}

function buildNav() {
  const nav = el("nav");
  nav.innerHTML = "";
  for (const page of PAGES) {
    const item = document.createElement("div");
    item.className = "nav-item";
    item.dataset.page = page.id;
    item.textContent = page.label;
    item.addEventListener("click", () => switchPage(page.id));
    nav.appendChild(item);
  }
}

function switchPage(id) {
  currentPage = id;
  for (const page of PAGES) {
    el(`page-${page.id}`).hidden = page.id !== id;
    const item = document.querySelector(`.nav-item[data-page="${page.id}"]`);
    item.classList.toggle("active", page.id === id);
  }
  if (id === "push") refreshPush();
  if (id === "rules") refreshRules();
  if (id === "settings") fillSettings();
}

function applyTheme() {
  const mode = config?.theme_mode || "system";
  const dark = mode === "dark" || (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

/* ---------- 推送页 ---------- */

async function refreshPush() {
  const messages = await invoke("get_messages");
  const table = el("push-table");
  const tbody = table.querySelector("tbody");
  tbody.innerHTML = "";
  if (!messages.length) {
    el("push-empty").hidden = false;
    table.hidden = true;
    return;
  }
  el("push-empty").hidden = true;
  table.hidden = false;
  const order =
    uiState?.column_order?.length > 0
      ? uiState.column_order
      : COLUMNS.map((c) => c.id);
  for (const m of messages) {
    const tr = document.createElement("tr");
    for (const value of cellValuesForOrder(order, m)) {
      const td = document.createElement("td");
      td.textContent = value;
      tr.appendChild(td);
    }
    tr.addEventListener("dblclick", () => navigator.clipboard.writeText(m.message || ""));
    tbody.appendChild(tr);
  }
}

function buildPushPage() {
  const page = el("page-push");
  page.innerHTML = `
    <div class="page-title">推送</div>
    <div class="toolbar">
      <button class="btn btn-secondary" id="btn-refresh">刷新</button>
      <button class="btn btn-secondary" id="btn-clear">清空</button>
    </div>
    <div class="table-wrap">
      <table id="push-table" hidden>
        <thead><tr id="push-head"></tr></thead>
        <tbody></tbody>
      </table>
    </div>
    <div class="empty" id="push-empty">暂无推送</div>
  `;
  pushTable = el("push-table");
  el("btn-refresh").addEventListener("click", refreshPush);
  el("btn-clear").addEventListener("click", async () => {
    if (await confirmBox("确定清空全部推送历史？此操作不可恢复。", "清空历史")) {
      await invoke("clear_history");
      refreshPush();
    }
  });
  const order = uiState?.column_order || COLUMNS.map((c) => c.id);
  const widths = uiState?.column_widths || {};
  const head = el("push-head");
  for (const id of order) {
    const col = COLUMNS.find((c) => c.id === id);
    const th = document.createElement("th");
    th.textContent = col ? col.title : id;
    th.dataset.col = id;
    if (widths[id]) th.style.width = `${widths[id]}px`;
    head.appendChild(th);
  }
  makeTableSortable();
}

/* SortableJS 列排序 + 右侧拖柄调整列宽 */
function makeTableSortable() {
  const table = pushTable;
  if (!table) return;

  table.querySelectorAll("th").forEach((th) => {
    // 右侧列宽拖柄（与列排序互不干扰）
    const handle = document.createElement("span");
    handle.className = "resize-handle";
    th.appendChild(handle);
    let resizing = false;
    const startResize = (e) => {
      if (resizing) return;
      resizing = true;
      e.preventDefault();
      e.stopImmediatePropagation();
      const id = th.dataset.col;
      const nextTh = th.nextElementSibling;
      const nextId = nextTh?.dataset.col ?? null;
      const startX = e.clientX;
      const startW = th.getBoundingClientRect().width;
      const startNext = nextTh ? nextTh.getBoundingClientRect().width : null;
      const minCurrent = MIN_COLUMN_WIDTHS[id] ?? 80;
      const minNext = nextId ? MIN_COLUMN_WIDTHS[nextId] ?? 80 : null;
      const onMove = (ev) => {
        if (!resizing) return;
        const { current, next } = resizeColumnBoundary(
          startW,
          startNext,
          ev.clientX - startX,
          minCurrent,
          minNext
        );
        th.style.width = `${Math.round(current)}px`;
        if (nextTh && next != null) {
          nextTh.style.width = `${Math.round(next)}px`;
        }
        if (!uiState) uiState = { column_order: [], column_widths: {} };
        uiState.column_widths[id] = Math.round(current);
        if (nextId && next != null) {
          uiState.column_widths[nextId] = Math.round(next);
        }
      };
      const onUp = () => {
        resizing = false;
        document.removeEventListener("pointermove", onMove);
        document.removeEventListener("pointerup", onUp);
        persistColumns();
      };
      document.addEventListener("pointermove", onMove);
      document.addEventListener("pointerup", onUp);
    };
    handle.addEventListener("pointerdown", startResize);
    handle.addEventListener("mousedown", startResize);
  });

  Sortable.create(
    table.tHead.rows[0],
    columnDragOptions((evt) => {
      const { oldIndex, newIndex } = evt;
      if (oldIndex == null || newIndex == null || oldIndex === newIndex) return;
      for (const row of table.tBodies[0].rows) {
        const ordered = moveInArray(Array.from(row.cells), oldIndex, newIndex);
        ordered.forEach((cell) => row.appendChild(cell));
      }
      persistColumns();
    })
  );
}

async function persistColumns() {
  const table = pushTable;
  if (!table) return;
  const order = Array.from(table.tHead.rows[0].cells).map((th) => th.dataset.col);
  const widths = uiState?.column_widths || {};
  uiState = await invoke("save_ui_state", { order, widths });
}

/* ---------- 规则页 ---------- */

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;"
  })[c]);
}

function buildRulesPage() {
  const page = el("page-rules");
  page.innerHTML = `
    <div class="page-title">规则</div>
    <div class="page-subtitle">按列表顺序匹配，优先命中生效 · 总开关在设置页</div>
    <div class="toolbar">
      <button class="btn btn-primary" id="btn-add-rule">添加规则</button>
    </div>
    <div class="card" id="rule-editor" hidden>
      <h3 id="rule-editor-title">新建规则</h3>
      <div class="field">
        <label>规则名称</label>
        <input type="text" id="rule-name" placeholder="如：银行验证码">
      </div>
      <div class="field">
        <label>触发关键词</label>
        <input type="text" id="rule-keywords" placeholder="多个关键词用逗号分隔，如：验证码, OTP">
      </div>
      <div class="row">
        <div class="field">
          <label>最小位数</label>
          <input type="number" id="rule-min" min="1" max="20">
        </div>
        <div class="field">
          <label>最大位数</label>
          <input type="number" id="rule-max" min="1" max="20">
        </div>
      </div>
      <div class="field">
        <label>匹配模式</label>
        <div class="segmented" id="rule-mode" data-value="both">
          <button type="button" class="seg-btn" data-value="keyword_only">关键词后</button>
          <button type="button" class="seg-btn" data-value="whole_text">全文</button>
          <button type="button" class="seg-btn" data-value="both">关键词后+全文回退</button>
        </div>
      </div>
      <div class="switch-row">
        <span>激活此规则</span>
        <label class="switch">
          <input type="checkbox" id="rule-enabled" checked>
          <span class="slider"></span>
        </label>
      </div>
      <div class="toolbar" style="justify-content:flex-end">
        <button class="btn btn-secondary" id="btn-rule-cancel">取消</button>
        <button class="btn btn-primary" id="btn-rule-save">保存</button>
      </div>
    </div>
    <div class="rule-list" id="rule-list" hidden></div>
    <div class="empty" id="rules-empty" hidden>暂无规则，点击"添加规则"创建</div>
    <div class="rule-hint" id="rules-hint">拖拽 ≡ 调整匹配优先级 · 停用或删除后即时生效</div>
  `;
  el("btn-add-rule").addEventListener("click", () => openEditor(null));
  el("btn-rule-cancel").addEventListener("click", closeEditor);
  el("btn-rule-save").addEventListener("click", saveRuleFromForm);
  el("rule-mode").querySelectorAll(".seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      el("rule-mode").dataset.value = btn.dataset.value;
      el("rule-mode").querySelectorAll(".seg-btn").forEach((b) => {
        b.classList.toggle("active", b === btn);
      });
    });
  });
  makeRuleListSortable();
}

function renderRules() {
  const list = el("rule-list");
  list.innerHTML = "";
  el("rules-empty").hidden = rules.length > 0;
  list.hidden = rules.length === 0;
  for (const rule of rules) {
    const row = document.createElement("div");
    row.className = "rule-row" + (rule.enabled ? "" : " off");
    row.dataset.id = rule.id;
    row.innerHTML = `
      <span class="rule-drag-handle">≡</span>
      <div class="rule-main">
        <div class="rule-name">${escapeHtml(rule.name)}</div>
        <div class="rule-summary">${escapeHtml(ruleSummary(rule))}</div>
      </div>
      <label class="switch">
        <input type="checkbox" class="rule-toggle" ${rule.enabled ? "checked" : ""}>
        <span class="slider"></span>
      </label>
      <div class="rule-actions">
        <button class="btn btn-secondary rule-edit">编辑</button>
        <button class="btn btn-danger rule-delete">删除</button>
      </div>
    `;
    row.querySelector(".rule-toggle").addEventListener("change", (e) =>
      toggleRule(rule.id, e.target.checked)
    );
    row.querySelector(".rule-edit").addEventListener("click", () => openEditor(rule));
    row.querySelector(".rule-delete").addEventListener("click", () => deleteRule(rule.id));
    list.appendChild(row);
  }
}

function makeRuleListSortable() {
  const list = el("rule-list");
  if (!list) return;
  Sortable.create(list, {
    animation: 150,
    draggable: ".rule-row",
    handle: ".rule-drag-handle",
    forceFallback: true,
    fallbackClass: "sortable-fallback",
    fallbackOnBody: true,
    ghostClass: "sortable-ghost",
    onEnd: async () => {
      const byId = new Map(rules.map((r) => [r.id, r]));
      const ordered = Array.from(list.querySelectorAll(".rule-row"))
        .map((row) => byId.get(row.dataset.id))
        .filter(Boolean);
      if (ordered.length === rules.length) {
        rules = ordered;
        await persistRules();
        renderRules();
      }
    }
  });
}

async function refreshRules() {
  rules = await invoke("get_rules");
  renderRules();
}

async function persistRules() {
  rules = await invoke("save_rules", { rules });
}

async function toggleRule(id, enabled) {
  rules = rules.map((r) => (r.id === id ? { ...r, enabled } : r));
  await persistRules();
  renderRules();
}

async function deleteRule(id) {
  const rule = rules.find((r) => r.id === id);
  if (!rule) return;
  if (!(await confirmBox(`确定删除规则"${rule.name}"？此操作不可恢复。`, "删除规则"))) return;
  rules = rules.filter((r) => r.id !== id);
  await persistRules();
  renderRules();
}

function openEditor(rule) {
  editingId = rule ? rule.id : null;
  el("rule-editor-title").textContent = rule ? "编辑规则" : "新建规则";
  el("rule-name").value = rule ? rule.name : "";
  el("rule-keywords").value = rule ? rule.keywords.join(", ") : "";
  el("rule-min").value = rule ? rule.min_length : 4;
  el("rule-max").value = rule ? rule.max_length : 8;
  const mode = rule ? rule.match_mode : "both";
  el("rule-mode").dataset.value = mode;
  el("rule-mode").querySelectorAll(".seg-btn").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.value === mode);
  });
  el("rule-enabled").checked = rule ? rule.enabled : true;
  el("rule-editor").hidden = false;
}

function closeEditor() {
  el("rule-editor").hidden = true;
  editingId = null;
}

async function saveRuleFromForm() {
  const draft = editingId ? rules.find((r) => r.id === editingId) : null;
  const rule = draft ? { ...draft } : newRule();
  rule.name = el("rule-name").value.trim();
  rule.keywords = parseKeywords(el("rule-keywords").value);
  rule.min_length = parseInt(el("rule-min").value, 10) || 1;
  rule.max_length = parseInt(el("rule-max").value, 10) || 1;
  rule.match_mode = el("rule-mode").dataset.value || "both";
  rule.enabled = el("rule-enabled").checked;
  const error = validateRule(rule);
  if (error) {
    await alertBox(error, "无法保存规则");
    return;
  }
  if (draft) {
    rules = rules.map((r) => (r.id === draft.id ? rule : r));
  } else {
    rules.push(rule);
  }
  await persistRules();
  closeEditor();
  renderRules();
}

/* ---------- 设置页 ---------- */

function fillSettings() {
  if (!config) return;
  el("set-server").value = config.server || "";
  el("set-username").value = config.username || "";
  el("set-password").value = config.password || "";
  el("set-topic").value = config.topic || "";
  const theme = config.theme_mode || "system";
  el("set-theme").dataset.value = theme;
  el("set-theme").querySelectorAll(".seg-btn").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.value === theme);
  });
  el("set-autostart").checked = !!config.auto_start;
  el("set-autocopy").checked = !!config.auto_copy_otp;
}

function buildSettingsPage() {
  const page = el("page-settings");
  page.innerHTML = `
    <div class="page-title">设置</div>
    <div class="page-subtitle">配置 ntfy-Notifier 连接参数</div>
    <div class="card">
      <h3>连接</h3>
      <div class="field"><label>服务器地址</label><input type="text" id="set-server" placeholder="https://..."></div>
      <div class="row">
        <div class="field"><label>用户名</label><input type="text" id="set-username"></div>
        <div class="field"><label>密码</label><input type="password" id="set-password"></div>
      </div>
      <div class="field"><label>主题</label><input type="text" id="set-topic" placeholder="your-topic"></div>
    </div>
    <div class="card">
      <h3>行为</h3>
      <div class="field">
        <label>界面主题</label>
        <div class="segmented" id="set-theme" data-value="system">
          <button type="button" class="seg-btn" data-value="system">跟随系统</button>
          <button type="button" class="seg-btn" data-value="light">浅色</button>
          <button type="button" class="seg-btn" data-value="dark">深色</button>
        </div>
      </div>
      <div class="switch-row">
        <span>开机自启动</span>
        <label class="switch">
          <input type="checkbox" id="set-autostart">
          <span class="slider"></span>
        </label>
      </div>
      <div class="switch-row">
        <span>收到短信时自动复制验证码到剪贴板</span>
        <label class="switch">
          <input type="checkbox" id="set-autocopy">
          <span class="slider"></span>
        </label>
      </div>
    </div>
    <div class="toolbar" style="justify-content:flex-end">
      <button class="btn btn-secondary" id="btn-cancel">取消</button>
      <button class="btn btn-primary" id="btn-save">保存</button>
    </div>
  `;
  el("btn-cancel").addEventListener("click", fillSettings);
  el("btn-save").addEventListener("click", async () => {
    const themeMode = el("set-theme").dataset.value || "system";
    const next = {
      ...config,
      server: el("set-server").value.trim(),
      username: el("set-username").value.trim(),
      password: el("set-password").value,
      topic: el("set-topic").value.trim(),
      theme_mode: themeMode,
      auto_start: el("set-autostart").checked,
      auto_copy_otp: el("set-autocopy").checked
    };
    if (next.server.startsWith("http://") && next.password) {
      if (!(await confirmBox("当前服务器地址使用 http://，密码将以明文在网络上传输，建议改用 https://。仍要保存吗？", "安全提示"))) return;
    }
    config = await invoke("save_config", { config: next });
    applyTheme();
  });
  el("set-theme").querySelectorAll(".seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      el("set-theme").dataset.value = btn.dataset.value;
      el("set-theme").querySelectorAll(".seg-btn").forEach((b) => {
        b.classList.toggle("active", b === btn);
      });
      // 即时预览主题
      const mode = btn.dataset.value;
      const dark =
        mode === "dark" ||
        (mode === "system" &&
          window.matchMedia("(prefers-color-scheme: dark)").matches);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
    });
  });
}

function buildAboutPage() {
  const page = el("page-about");
  page.innerHTML = `
    <div class="page-title">ntfy-Notifier</div>
    <div class="page-subtitle">版本 1.0.0（Rust/Tauri）</div>
    <div class="card">
      <p>Windows 系统托盘工具，订阅 ntfy 消息并弹出系统通知。</p>
      <p style="margin-top:8px"><a href="https://github.com/a01lu01/ntfy-notifier" target="_blank">https://github.com/a01lu01/ntfy-notifier</a></p>
    </div>
  `;
}

/* ---------- 初始化 ---------- */

async function init() {
  config = await invoke("get_config");
  uiState = await invoke("get_ui_state");
  applyTheme();
  buildNav();
  buildPushPage();
  buildRulesPage();
  buildSettingsPage();
  buildAboutPage();
  switchPage("push");

  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", applyTheme);

  await listen("navigate", (e) => switchPage(e.payload));
  await listen("history-updated", () => { if (currentPage === "push") refreshPush(); });
}

init().catch((e) => console.error(e));
