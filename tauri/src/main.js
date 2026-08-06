import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Sortable from "sortablejs";
import { cellValuesForOrder, moveInArray } from "./table-model.js";
import { columnDragOptions } from "./table-drag.js";

const PAGES = [
  { id: "push", label: "推送" },
  { id: "settings", label: "设置" },
  { id: "about", label: "关于" }
];

let currentPage = "push";
let config = null;
let uiState = null;
let pushTable = null;

const COLUMNS = [
  { id: "time", title: "时间" },
  { id: "title", title: "标题" },
  { id: "message", title: "内容" }
];

function el(id) { return document.getElementById(id); }

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
    if (confirm("确定清空全部推送历史？此操作不可恢复。")) {
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
      const startX = e.clientX;
      const startW = th.getBoundingClientRect().width;
      const onMove = (ev) => {
        if (!resizing) return;
        const w = Math.max(80, Math.round(startW + ev.clientX - startX));
        th.style.width = `${w}px`;
        if (!uiState) uiState = { column_order: [], column_widths: {} };
        uiState.column_widths[id] = w;
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
      <div class="field"><label>主题</label><input type="text" id="set-topic" placeholder="sms"></div>
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
      if (!confirm("当前服务器地址使用 http://，密码将以明文在网络上传输，建议改用 https://。仍要保存吗？")) return;
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
  buildSettingsPage();
  buildAboutPage();
  switchPage("push");

  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", applyTheme);

  await listen("navigate", (e) => switchPage(e.payload));
  await listen("history-updated", () => { if (currentPage === "push") refreshPush(); });
}

init().catch((e) => console.error(e));
