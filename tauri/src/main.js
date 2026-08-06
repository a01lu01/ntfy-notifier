import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const PAGES = [
  { id: "push", label: "推送" },
  { id: "settings", label: "设置" },
  { id: "about", label: "关于" }
];

let currentPage = "push";
let config = null;
let uiState = null;

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
  for (const m of messages) {
    const tr = document.createElement("tr");
    tr.innerHTML = `<td>${m.time}</td><td>${m.title || ""}</td><td>${m.message || ""}</td>`;
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

/* 列拖拽排序（简化版：拖动表头交换列） */
function makeTableSortable() {
  const table = el("push-table");
  let dragCol = null;
  table.querySelectorAll("th").forEach((th, index) => {
    th.addEventListener("mousedown", () => { dragCol = index; });
    th.addEventListener("mouseenter", () => {
      if (dragCol !== null && dragCol !== index) {
        const head = table.tHead.rows[0];
        const cells = Array.from(head.cells);
        if (dragCol < cells.length && index < cells.length) {
          head.insertBefore(cells[dragCol], cells[index + (index > dragCol ? 1 : 0)]);
          dragCol = index;
        }
      }
    });
  });
  document.addEventListener("mouseup", async () => {
    if (dragCol !== null) {
      dragCol = null;
      const order = Array.from(table.tHead.rows[0].cells).map((th) => th.dataset.col);
      const widths = uiState?.column_widths || {};
      uiState = await invoke("save_ui_state", { order, widths });
    }
  });
}

/* ---------- 设置页 ---------- */

function fillSettings() {
  if (!config) return;
  el("set-server").value = config.server || "";
  el("set-username").value = config.username || "";
  el("set-password").value = config.password || "";
  el("set-topic").value = config.topic || "";
  el("set-theme").value = config.theme_mode || "system";
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
        <select id="set-theme">
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </div>
      <div class="checkbox-row"><input type="checkbox" id="set-autostart"><label for="set-autostart">开机自启动</label></div>
      <div class="checkbox-row"><input type="checkbox" id="set-autocopy"><label for="set-autocopy">收到短信时自动复制验证码到剪贴板</label></div>
    </div>
    <div class="toolbar" style="justify-content:flex-end">
      <button class="btn btn-secondary" id="btn-cancel">取消</button>
      <button class="btn btn-primary" id="btn-save">保存</button>
    </div>
  `;
  el("btn-cancel").addEventListener("click", fillSettings);
  el("btn-save").addEventListener("click", async () => {
    const next = {
      ...config,
      server: el("set-server").value.trim(),
      username: el("set-username").value.trim(),
      password: el("set-password").value,
      topic: el("set-topic").value.trim(),
      theme_mode: el("set-theme").value,
      auto_start: el("set-autostart").checked,
      auto_copy_otp: el("set-autocopy").checked
    };
    if (next.server.startsWith("http://") && next.password) {
      if (!confirm("当前服务器地址使用 http://，密码将以明文在网络上传输，建议改用 https://。仍要保存吗？")) return;
    }
    config = await invoke("save_config", { config: next });
    applyTheme();
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
