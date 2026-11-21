// ManualBundler 前端主逻辑 —— 普通脚本模式（非 module）
// ========== Tauri API 层：全局 API + 降级模式 ==========
const __TAURI__ = typeof window !== 'undefined' ? (window.__TAURI__ || null) : null;
const tauriAvailable = !!__TAURI__;

// invoke: 调用 Rust 端命令（Tauri 2.x: window.__TAURI__.core.invoke）
async function invoke(cmd, args) {
  if (tauriAvailable && __TAURI__.core && typeof __TAURI__.core.invoke === 'function') {
    return await __TAURI__.core.invoke(cmd, args);
  }
  console.warn('[MOCK invoke]', cmd, args);
  return null;
}

// listen: 监听 Tauri 事件
async function listen(eventName, handler) {
  if (tauriAvailable && __TAURI__.event && typeof __TAURI__.event.listen === 'function') {
    return await __TAURI__.event.listen(eventName, handler);
  }
  console.warn('[MOCK listen]', eventName);
  return null;
}

// open: dialog —— 打开文件/文件夹选择器
async function open(options) {
  if (tauriAvailable && __TAURI__.dialog && typeof __TAURI__.dialog.open === 'function') {
    return await __TAURI__.dialog.open(options);
  }
  console.warn('[MOCK open]', options);
  const path = prompt('[浏览器模式] 请输入目录路径:', '');
  return path || null;
}

// save: dialog —— 保存文件选择器
async function save(options) {
  if (tauriAvailable && __TAURI__.dialog && typeof __TAURI__.dialog.save === 'function') {
    return await __TAURI__.dialog.save(options);
  }
  console.warn('[MOCK save]', options);
  const path = prompt('[浏览器模式] 请输入保存路径:', '');
  return path || null;
}

// ========== 全局配置状态 ==========
const State = {
  config: {
    input_dirs: [],
    title: "泳和批量改名",
    breadcrumb_prefix: "泳和批量改名 / 用户手册 /",
    nav: [],
    shell_style: "C",
    options: { merge_css: true, collapsible_sidebar: true, aux_lang: null },
    output_path: null,
    locale: null,
  },
  scanResult: null,
  scanDirMap: new Map(), // 输入目录路径 -> 本次扫描的 pages/dir_tree 引用（移除目录时用于清理）
  pageIdMap: {}, // file -> page_id
  draggedItem: null, // 拖拽中元素
  lastPreviewTime: 0,
  lastStats: null, // 最近一次预览统计（用于体积性能提示与导出确认）
};

// ========== 日志面板 ==========
const LogPanel = {
  entries: [],
  errorCount: 0,
  enabled: true,
  autoScroll: true,

  add(level, source, message) {
    const entry = {
      time: new Date().toLocaleTimeString('zh-CN', { hour12: false }),
      level, source, message,
    };
    this.entries.push(entry);
    if (level === 'ERROR') this.errorCount++;
    this.render();
    if (level === 'ERROR') showToast(message, 'error');
  },

  render() {
    const list = document.getElementById('log-list');
    const summary = document.getElementById('log-summary');
    const badge = document.getElementById('log-error-badge');

    if (this.enabled) {
      // 只渲染最后 200 条
      const toShow = this.entries.slice(-200);
      list.innerHTML = toShow.map((e, i) =>
        `<div class="log-entry level-${e.level}" data-idx="${this.entries.length - toShow.length + i}" title="${escapeAttr(e.message)}">
          <span class="log-time">${e.time}</span>
          <span class="log-level">${e.level}</span>
          <span class="log-source">${e.source}</span>
          <span class="log-message">${escapeHtml(e.message)}</span>
        </div>`
      ).join('');
      if (this.autoScroll && list.parentElement && list.parentElement.scrollTop !== undefined) {
        list.parentElement.scrollTop = list.parentElement.scrollHeight;
      }
    }

    // 顶栏摘要
    const last = this.entries[this.entries.length - 1];
    if (last) {
      summary.textContent = `${last.time} [${last.level}] ${last.source}: ${last.message.substring(0, 80)}`;
    }
    if (this.errorCount > 0) {
      badge.style.display = 'inline-block';
      badge.textContent = this.errorCount;
    } else {
      badge.style.display = 'none';
    }
  },

  clear() {
    this.entries = [];
    this.errorCount = 0;
    this.render();
  },

  copyAll() {
    const text = this.entries.map(e => `${e.time} ${e.level} [${e.source}] ${e.message}`).join('\n');
    navigator.clipboard.writeText(text);
  },
};

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function escapeAttr(s) { return escapeHtml(s); }

// ========== Toast ==========
function showToast(msg, type) {
  type = type || 'info';
  const container = document.getElementById('toast-container');
  const t = document.createElement('div');
  t.className = `toast ${type}`;
  t.textContent = msg;
  container.appendChild(t);
  setTimeout(() => {
    t.style.animation = 'fadeOut .3s ease forwards';
    setTimeout(() => t.remove(), 300);
  }, 8000);
  t.addEventListener('click', () => t.remove());
}

// ========== 全局异常捕获 v2.4 ==========
window.addEventListener('error', (e) => {
  LogPanel.add('ERROR', '前端', t('log.uncaughtError', { msg: e.message, file: e.filename, line: e.lineno }));
});
window.addEventListener('unhandledrejection', (e) => {
  LogPanel.add('ERROR', '前端', t('log.uncaughtPromise', { reason: String(e.reason) }));
});

// ========== Tauri 事件监听 ==========
async function setupListeners() {
  if (!tauriAvailable) return;
  try {
    await listen('mb-log', (event) => {
      const d = event.payload;
      LogPanel.add(d.level || 'INFO', d.source || '', d.message || '');
    });
  } catch (e) {
    console.warn('Event listen failed:', e);
  }
}

// ========== 输入目录按钮 ==========
async function onOpenFolder() {
  try {
    const selected = await open({ directory: true, multiple: false });
    if (!selected) return;
    await addInputDir(selected);
  } catch (e) {
    LogPanel.add('ERROR', '前端', `打开文件夹失败: ${e}`);
  }
}

// ========== 顶栏/输出按钮 ==========
// 「打开输出文件夹」：在系统文件管理器中定位输出文件（未设置输出路径时提示）
async function onOpenOutputFolder() {
  try {
    const p = State.config.output_path;
    if (!p) {
      showToast(t('toast.noOutputPath'), 'warn');
      return;
    }
    const result = await invoke('open_output_folder', { path: p });
    if (result === null) {
      showToast(t('toast.mockBundle'), 'warn');
    }
  } catch (e) {
    LogPanel.add('ERROR', '输出', t('toast.openOutputFail', { err: e.message || e }));
    showToast(t('toast.openOutputFail', { err: e.message || e }), 'error');
  }
}

async function addInputDir(path) {
  if (State.config.input_dirs.includes(path)) return;
  State.config.input_dirs.push(path);
  renderInputDirs();

  // 触发扫描
  try {
    const result = await invoke('scan_folder', { path: path });
    if (!result) {
      LogPanel.add('WARN', '扫描', 'invoke 返回 null（可能是浏览器降级模式）');
      if (!State.scanResult) State.scanResult = { pages: [], dir_tree: [] };
      renderPageList(); renderNavTree(); return;
    }
    // 累积合并到 scanResult（用户可能添加多个目录）
    if (!State.scanResult) State.scanResult = { pages: [], dir_tree: [] };
    // 按 path 去重合并 pages
    const existingPaths = new Set(State.scanResult.pages.map(p => p.path));
    result.pages.forEach(p => { if (!existingPaths.has(p.path)) State.scanResult.pages.push(p); });
    // dir_tree 简单追加（后续可能要递归合并）
    result.dir_tree.forEach(t => State.scanResult.dir_tree.push(t));
    // 记录该目录的扫描结果引用，供移除目录时清理
    State.scanDirMap.set(path, { pages: result.pages, tree: result.dir_tree });
    renderPageList();
    renderNavTree();
    LogPanel.add('INFO', '扫描', `扫描完成: ${result.pages.length} 页（累计 ${State.scanResult.pages.length} 页）`);
    schedulePreview();
  } catch (e) {
    LogPanel.add('ERROR', '扫描', String(e));
  }
}

// ========== 输出路径 ==========
function renderOutputPath() {
  const el = document.getElementById('output-path');
  if (el) el.textContent = State.config.output_path || '';
}

async function onSelectOutput() {
  try {
    const p = await save({
      defaultPath: (State.config.title || 'manual') + '.html',
      filters: [{ name: 'HTML', extensions: ['html'] }],
    });
    if (!p) return;
    State.config.output_path = p;
    renderOutputPath();
    LogPanel.add('INFO', '输出', t('log.outputSet', { path: p }));
    showToast(t('toast.outputSet'), 'info');
  } catch (e) {
    LogPanel.add('ERROR', '输出', String(e));
    showToast(t('toast.outputSetFail', { err: e.message }), 'error');
  }
}

async function onGenerate() {
  try {
    if (State.config.input_dirs.length === 0) {
      showToast(t('toast.addInputFirst'), 'warn');
      return;
    }
    // ≥20MB：导出前弹出拆分建议确认框（继续 / 取消）
    if (State.lastStats && State.lastStats.final_bytes >= 20 * 1024 * 1024) {
      const go = await askSplitConfirm();
      if (!go) {
        LogPanel.add('INFO', '打包', t('log.exportCancelled'));
        return;
      }
    }
    // 已设置输出路径则直接使用，否则弹出保存对话框并记住
    let savePath = State.config.output_path;
    if (!savePath) {
      const defaultName = (State.config.title || 'manual') + '.html';
      savePath = await save({
        defaultPath: defaultName,
        filters: [{ name: 'HTML', extensions: ['html'] }],
      });
      if (!savePath) return;
      State.config.output_path = savePath;
      renderOutputPath();
    }
    const result = await invoke('write_bundle', { config: State.config, savePath: savePath });
    if (!result) { showToast(t('toast.mockBundle'), 'warn'); return; }
    LogPanel.add('INFO', '打包', t('log.generateDone', { path: result.path, size: formatSize(result.bytes) }));
    showToast(t('toast.generateOk', { size: formatSize(result.bytes) }), 'info');
  } catch (e) {
    LogPanel.add('ERROR', '打包', String(e));
    showToast(t('toast.generateFail'), 'error');
  }
}

// ========== 预览（右栏实时预览，无独立按钮；输入/导航/配置变更自动重打包） ==========
let previewTimer = null;
function schedulePreview() {
  if (previewTimer) clearTimeout(previewTimer);
  previewTimer = setTimeout(doPreview, 300);
}

let previewRetryCount = 0;
async function doPreview() {
  const iframe = document.getElementById('preview-iframe');
  if (State.config.input_dirs.length === 0) {
    // 空输入：显示实时预览占位提示
    iframe.srcdoc = `<p style="padding:24px;color:#8b93a7;font-family:sans-serif;font-size:13px;line-height:1.6">${escapeHtml(t('hint.previewEmpty'))}</p>`;
    return;
  }
  try {
    const result = await invoke('preview_bundle', { config: State.config });
    if (!result) {
      iframe.srcdoc = '<p style="padding:20px;color:#888">浏览器降级模式 · 请在 Tauri 中运行以查看预览</p>';
      return;
    }
    iframe.srcdoc = result.html;
    updateStats(result.stats);
    handleMissingResources(result.missing_resources);
    previewRetryCount = 0;
  } catch (e) {
    LogPanel.add('ERROR', '前端', `预览失败: ${e}`);
    if (previewRetryCount < 1) {
      previewRetryCount++;
      setTimeout(doPreview, 500);
    } else {
      showToast(t('toast.previewFail'), 'error');
    }
  }
}

function updateStats(stats) {
  State.lastStats = stats;
  document.getElementById('stat-pages').textContent = stats.page_count;
  document.getElementById('stat-resources').textContent = stats.embedded_resource_count;
  const vol = document.getElementById('stat-volume');
  vol.textContent = formatSize(stats.final_bytes);
  // 体积性能提示：<10M 绿 / 10~20M 黄 / ≥20M 红
  vol.classList.remove('size-ok', 'size-warn', 'size-bad');
  vol.classList.add(sizeLevelClass(stats.final_bytes));
}

// 体积分级：<10MB 绿(ok)、10~20MB 黄(warn)、≥20MB 红(bad)
function sizeLevelClass(bytes) {
  const MB = 1024 * 1024;
  if (bytes >= 20 * MB) return 'size-bad';
  if (bytes >= 10 * MB) return 'size-warn';
  return 'size-ok';
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

// ========== 左栏渲染 ==========
function renderInputDirs() {
  const container = document.getElementById('input-dirs');
  container.innerHTML = State.config.input_dirs.map((d, i) => `
    <div class="input-dir-item">
      <span title="${escapeAttr(d)}">📁 ${escapeHtml(d.split(/[\\/]/).pop() || d)}</span>
      <button class="remove-btn" data-idx="${i}" title="移除">×</button>
    </div>
  `).join('');
  container.querySelectorAll('.remove-btn').forEach(btn => {
    btn.onclick = () => removeInputDir(parseInt(btn.dataset.idx));
  });
}

// 移除输入目录：同步清理其页面、目录树条目及引用这些页面的导航节点
function removeInputDir(idx) {
  const dir = State.config.input_dirs.splice(idx, 1)[0];
  if (!dir) return;
  const record = State.scanDirMap.get(dir);
  State.scanDirMap.delete(dir);

  let removedPages = 0;
  let prunedNav = 0;
  if (record && State.scanResult) {
    const pageSet = new Set(record.pages);
    const before = State.scanResult.pages.length;
    State.scanResult.pages = State.scanResult.pages.filter(p => !pageSet.has(p));
    removedPages = before - State.scanResult.pages.length;

    const treeSet = new Set(record.tree);
    State.scanResult.dir_tree = State.scanResult.dir_tree.filter(t => !treeSet.has(t));

    // 清理导航中引用了已移除页面文件的节点，避免打包时缺页
    const files = new Set(record.pages.map(p => p.path));
    prunedNav = pruneNavByFiles(files);
  }

  if (dir) LogPanel.add('INFO', '输入', t('log.dirRemoved', {
    dir,
    pages: removedPages,
    nav: prunedNav ? t('log.navNodesSuffix', { count: prunedNav }) : ''
  }));
  renderInputDirs();
  renderPageList();
  renderNavTree();
  schedulePreview();
}

// 递归移除 nav 中 file 命中 filesSet 的页面节点，返回移除数量
function pruneNavByFiles(filesSet) {
  let removed = 0;
  const walk = (nodes) => nodes.filter(n => {
    if (n.kind === 'page' && filesSet.has(n.file)) { removed++; return false; }
    if (n.children && n.children.length > 0) n.children = walk(n.children);
    return true;
  });
  State.config.nav = walk(State.config.nav);
  return removed;
}

function renderPageList() {
  const container = document.getElementById('page-list');
  if (!State.scanResult || State.scanResult.pages.length === 0) {
    container.innerHTML = `<div class="empty-hint">${t('hint.pageListEmpty')}</div>`;
    return;
  }
  const iconFor = (p) => {
    const m = String(p.path || '').toLowerCase().match(/\.([a-z0-9]+)$/);
    const ext = m ? m[1] : '';
    if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg', 'ico', 'avif'].includes(ext)) return '🖼';
    if (ext === 'mht' || ext === 'mhtml') return '🌐';
    if (['pdf', 'doc', 'docx', 'rtf', 'wps', 'odt', 'odp', 'ppt', 'pptx', 'txt', 'md'].includes(ext)) return '📑';
    return '📄';
  };
  container.innerHTML = State.scanResult.pages.map((p, i) => `
    <div class="page-item" draggable="true" data-file="${escapeAttr(p.path)}" data-name="${escapeAttr(p.title || p.path)}">
      <span class="page-icon">${iconFor(p)}</span>
      <span>${escapeHtml(p.title || p.path)}</span>
      <button class="remove-btn" data-idx="${i}" title="${escapeAttr(t('btn.removePage'))}">×</button>
    </div>
  `).join('');
  container.querySelectorAll('.page-item .remove-btn').forEach(btn => {
    btn.onclick = (e) => { e.stopPropagation(); removePage(parseInt(btn.dataset.idx)); };
  });
  container.querySelectorAll('.page-item').forEach(el => {
    el.addEventListener('dragstart', (e) => {
      State.draggedItem = { source: 'page-list', file: el.dataset.file, name: el.dataset.name };
      el.classList.add('dragging');
      e.dataTransfer.effectAllowed = 'copy';
      // HTML5 规范要求：必须 setData 才能让 Chromium/WebView2 派发 dragover/drop
      e.dataTransfer.setData('text/plain', `page-list:${el.dataset.file}`);
    });
    el.addEventListener('dragend', () => {
      el.classList.remove('dragging');
      State.draggedItem = null;
    });
  });
}

// 移除单个页面：同步清理导航中引用该页的节点
function removePage(idx) {
  const p = State.scanResult && State.scanResult.pages[idx];
  if (!p) return;
  State.scanResult.pages.splice(idx, 1);
  pruneNavByFiles(new Set([p.path]));
  LogPanel.add('INFO', '输入', t('log.pageRemoved', { name: p.title || p.path }));
  renderPageList();
  renderNavTree();
  schedulePreview();
}

// ========== 导航树渲染与拖拽（完整重写 v2 — 修复绑定时机） ==========
function renderNavTree() {
  const container = document.getElementById('nav-tree');
  // 工具栏按钮栏
  const toolbarHtml = `
    <div class="nav-toolbar">
      <button class="btn-mini" data-act="addGroup">${t('btn.group')}</button>
      <button class="btn-mini" data-act="addPage">${t('btn.page')}</button>
      <button class="btn-mini" data-act="delete">${t('btn.delete')}</button>
      <div class="sep"></div>
      <button class="btn-mini" data-act="importNav">${t('btn.import')}</button>
      <button class="btn-mini" data-act="exportNav">${t('btn.export')}</button>
    </div>
  `;

  // 先注入 HTML，再绑定事件
  if (State.config.nav.length === 0) {
    container.innerHTML = toolbarHtml +
      `<div class="empty-hint" style="padding:24px 10px">${t('hint.navTreeEmpty')}</div>`;
  } else {
    container.innerHTML = toolbarHtml +
      State.config.nav.map((node, idx) => renderNode(node, [idx])).join('');
  }

  // 工具栏事件（此时 DOM 已存在）
  container.querySelectorAll('.nav-toolbar button').forEach(btn => {
    btn.onclick = (e) => { e.stopPropagation(); handleNavToolbar(btn.dataset.act); };
  });

  if (State.config.nav.length > 0) {
    bindNavTreeEvents(container);
  }
  bindNavRootDrop();
}

// ========== 导航工具栏处理 ==========
function handleNavToolbar(act) {
  if (act === 'addGroup') {
    const name = prompt(t('prompt.groupName'), t('default.groupName'));
    if (!name) return;
    const path = State.selectedNavPath;
    const newNode = { kind: 'text', name: name, children: [] };
    if (path && path.length > 0) {
      const parent = getNodeByPath(State.config.nav, path);
      if (parent) {
        parent.children = parent.children || [];
        parent.children.push(newNode);
      } else {
        State.config.nav.push(newNode);
      }
    } else {
      State.config.nav.push(newNode);
    }
    LogPanel.add('INFO', '导航', t('log.navGroupAdded', { name }));
    renderNavTree(); schedulePreview();
  }
  else if (act === 'addPage') {
    const name = prompt(t('prompt.pageName'), t('default.pageName'));
    if (!name) return;
    const file = prompt(t('prompt.pageFile'), '') || '';
    const path = State.selectedNavPath;
    const newNode = { kind: 'page', name: name, file: file, children: [] };
    if (path && path.length > 0) {
      const parent = getNodeByPath(State.config.nav, path);
      if (parent) {
        parent.children = parent.children || [];
        parent.children.push(newNode);
      } else {
        State.config.nav.push(newNode);
      }
    } else {
      State.config.nav.push(newNode);
    }
    LogPanel.add('INFO', '导航', t('log.navPageAdded', { name, file }));
    renderNavTree(); schedulePreview();
  }
  else if (act === 'delete') {
    if (!State.selectedNavPath || State.selectedNavPath.length === 0) {
      showToast(t('warn.selectNodeFirst'), 'warn'); return;
    }
    const node = getNodeByPath(State.config.nav, State.selectedNavPath);
    if (!node) return;
    const childSuffix = node.children && node.children.length > 0
      ? t('confirm.deleteChildrenSuffix', { count: node.children.length }) : '';
    if (!confirm(t('confirm.deleteNodeByName', { name: node.name, children: childSuffix }))) return;
    deleteNodeByPath(State.selectedNavPath);
    State.selectedNavPath = null;
    LogPanel.add('INFO', '导航', t('log.navNodeDeleted', { name: node.name }));
    renderNavTree(); schedulePreview();
  }
  else if (act === 'importNav') {
    importNav();
  }
  else if (act === 'exportNav') {
    exportNav();
  }
}

// 兼容 index.html 里 btn-add-group 的旧绑定
function onAddGroup() { handleNavToolbar('addGroup'); }

async function importNav() {
  try {
    const path = await open({
      multiple: false,
      filters: [
        { name: '项目/导航', extensions: ['mproj', 'nav', 'json'] },
        { name: '所有文件', extensions: ['*'] },
      ],
    });
    if (!path) return;
    let config = null;
    if (path.toLowerCase().endsWith('.mproj')) {
      config = await invoke('load_project', { path: path });
      if (!config) { showToast(t('toast.projLoadFail'), 'error'); return; }
    } else {
      // 纯 JSON 导入 — 读文件内容
      const content = await invoke('read_text_file', { path: path });
      if (!content) { showToast(t('toast.projReadFail'), 'error'); return; }
      config = JSON.parse(content);
    }
    if (config.nav && Array.isArray(config.nav)) {
      State.config.nav = config.nav;
      LogPanel.add('INFO', '导航', t('log.navImportOk', { count: config.nav.length }));
      renderNavTree(); schedulePreview();
      showToast(t('toast.navImported'), 'info');
    } else {
      showToast(t('toast.navNoData'), 'warn');
    }
  } catch (e) {
    LogPanel.add('ERROR', '导航导入', String(e));
    showToast(t('toast.importFail', { err: e.message }), 'error');
  }
}

async function exportNav() {
  try {
    if (State.config.nav.length === 0) {
      showToast(t('warn.navEmpty'), 'warn'); return;
    }
    const savePath = await save({
      defaultPath: 'navigation.nav.json',
      filters: [
        { name: '导航 JSON', extensions: ['nav.json', 'nav', 'json'] },
      ],
    });
    if (!savePath) return;
    const data = JSON.stringify(State.config.nav, null, 2);
    await invoke('write_text_file', { path: savePath, content: data });
    LogPanel.add('INFO', '导航导出', t('log.navExportOk', { count: State.config.nav.length, path: savePath }));
    showToast(t('toast.navExported'), 'info');
  } catch (e) {
    LogPanel.add('ERROR', '导航导出', String(e));
    showToast(t('toast.exportFail', { err: e.message }), 'error');
  }
}

function renderNode(node, path) {
  const hasPage = node.kind === 'page';
  const cls = hasPage ? 'node-page' : 'node-text';
  const childrenHtml = node.children && node.children.length > 0
    ? `<div class="nav-node-children">${node.children.map(function(c, i) { return renderNode(c, path.concat([i])); }).join('')}</div>`
    : '';
  return `
    <div class="nav-node ${cls}" draggable="true" data-path="${path.join('.')}" data-kind="${node.kind}">
      <span>${escapeHtml(node.name)}</span>
      ${hasPage && node.file ? `<span style="margin-left:auto;font-size:10px;color:var(--dim)">${escapeHtml(node.file)}</span>` : ''}
    </div>
    ${childrenHtml}
  `;
}

let navRootDropBound = false;
function bindNavRootDrop() {
  if (navRootDropBound) return; // 只绑定一次，避免 renderNavTree 叠加监听器
  navRootDropBound = true;
  const container = document.getElementById('nav-tree');
  container.addEventListener('dragover', (e) => {
    const dragged = State.draggedItem;
    if (!dragged) return;
    // 只有拖到空白区（没 over 到具体 node）才显示根级 drop 高亮
    if (!e.target.closest('.nav-node')) {
      e.preventDefault();
      container.classList.add('drop-target');
    }
  });
  container.addEventListener('dragleave', (e) => {
    if (e.target === container) container.classList.remove('drop-target');
  });
  container.addEventListener('drop', (e) => {
    const dragged = State.draggedItem;
    if (!dragged) return;
    if (e.target === container || !e.target.closest('.nav-node')) {
      e.preventDefault();
      container.classList.remove('drop-target');
      handleNavRootDrop(dragged);
    }
  });
}

function bindNavTreeEvents(container) {
  // 拖拽 —— 三类落点判定
  container.querySelectorAll('.nav-node').forEach(el => {
    el.addEventListener('dragstart', (e) => {
      // 关键 guard：如果已经有跨面板拖拽（page-list 来源），不要覆盖！
      // 否则用户从左栏拖到 nav-node 时，nav-node 会抢 State.draggedItem
      if (State.draggedItem && State.draggedItem.source === 'page-list') {
        // 不让 nav-node 的 dragstart 覆盖 page-list 的拖拽
        e.preventDefault();
        return;
      }
      const path = el.dataset.path.split('.').map(Number);
      State.draggedItem = { source: 'nav-tree', path, kind: el.dataset.kind };
      el.classList.add('dragging');
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', path.join(','));
      clearDropIndicators(container);
    });
    el.addEventListener('dragend', () => {
      el.classList.remove('dragging');
      State.draggedItem = null;
      clearDropIndicators(container);
      document.getElementById('nav-tree').classList.remove('drop-target');
    });
    el.addEventListener('dragover', (e) => {
      const dragged = State.draggedItem;
      if (!dragged) return;
      e.preventDefault();

      // 防自循环：如果目标节点是源节点的子孙，禁止 drop
      const targetPath = el.dataset.path.split('.').map(Number);
      if (dragged.source === 'nav-tree' && isDescendant(dragged.path, targetPath)) {
        clearDropIndicators(container);
        el.classList.add('drop-forbidden');
        e.dataTransfer.dropEffect = 'none';
        return;
      }

      // 根据光标位置判定落点（上1/3=before，下1/3=after，中间=inside）
      clearDropIndicators(container);
      const rect = el.getBoundingClientRect();
      const y = e.clientY - rect.top;
      const rel = y / rect.height;

      if (dragged.source === 'page-list') {
        // 从左栏拖入：只能 inside（成为节点）
        el.classList.add('drop-inside');
        e.dataTransfer.dropEffect = 'copy';
      } else {
        if (rel < 0.33) {
          el.classList.add('drop-before');
          e.dataTransfer.dropEffect = 'move';
        } else if (rel > 0.67) {
          el.classList.add('drop-after');
          e.dataTransfer.dropEffect = 'move';
        } else {
          el.classList.add('drop-inside');
          e.dataTransfer.dropEffect = 'move';
        }
      }
    });
    el.addEventListener('dragleave', (e) => {
      // 只当离开整个元素时清除指示
      if (!el.contains(e.relatedTarget)) {
        el.classList.remove('drop-before', 'drop-after', 'drop-inside', 'drop-forbidden');
      }
    });
    el.addEventListener('drop', (e) => {
      const dragged = State.draggedItem;
      LogPanel.add('DEBUG', '拖拽', t('log.dropDebug', { source: dragged ? dragged.source : 'null', target: el.dataset.path }));
      if (!dragged) return;
      e.preventDefault();
      clearDropIndicators(container);
      const targetPath = el.dataset.path.split('.').map(Number);
      if (dragged.source === 'nav-tree' && isDescendant(dragged.path, targetPath)) return;

      // 判定落点类型
      const rect = el.getBoundingClientRect();
      const y = e.clientY - rect.top;
      const rel = y / rect.height;
      let position = 'inside';
      if (dragged.source === 'nav-tree') {
        if (rel < 0.33) position = 'before';
        else if (rel > 0.67) position = 'after';
      }

      handleNavDrop(dragged, targetPath, position);
    });
  });

  // 点击选中 + 双击改名 + 右键菜单
  container.querySelectorAll('.nav-node').forEach(el => {
    el.addEventListener('click', () => {
      container.querySelectorAll('.nav-node.selected').forEach(n => n.classList.remove('selected'));
      el.classList.add('selected');
      State.selectedNavPath = el.dataset.path.split('.').map(Number);
      closeCtxMenu();
    });
    el.addEventListener('dblclick', (e) => {
      e.stopPropagation();
      const path = el.dataset.path.split('.').map(Number);
      const node = getNodeByPath(State.config.nav, path);
      if (!node) return;
      // 内联改名
      const span = el.querySelector('span');
      if (!span) return;
      const input = document.createElement('input');
      input.type = 'text'; input.value = node.name;
      input.className = 'nav-rename-input';
      input.style.cssText = 'flex:1;background:var(--bg-dark);border:1px solid var(--cyan-b);border-radius:3px;color:var(--txt);padding:2px 4px;font-size:12px;outline:none;';
      span.replaceWith(input); input.focus(); input.select();
      const commit = () => {
        const v = input.value.trim() || node.name;
        node.name = v; renderNavTree(); schedulePreview();
      };
      input.addEventListener('blur', commit);
      input.addEventListener('keydown', (ev) => {
        if (ev.key === 'Enter') input.blur();
        else if (ev.key === 'Escape') { input.value = node.name; input.blur(); }
      });
    });
    el.addEventListener('contextmenu', (e) => {
      e.preventDefault(); e.stopPropagation();
      const path = el.dataset.path.split('.').map(Number);
      openCtxMenu(e.clientX, e.clientY, path);
    });
  });
}

function clearDropIndicators(container) {
  container.querySelectorAll('.nav-node').forEach(n => {
    n.classList.remove('drop-before', 'drop-after', 'drop-inside', 'drop-forbidden');
  });
}

function isDescendant(ancestorPath, childPath) {
  if (childPath.length <= ancestorPath.length) return false;
  for (let i = 0; i < ancestorPath.length; i++) {
    if (childPath[i] !== ancestorPath[i]) return false;
  }
  return true;
}

function handleNavRootDrop(dragged) {
  if (dragged.source === 'page-list') {
    State.config.nav.push({ kind: 'page', name: dragged.name, file: dragged.file, children: [] });
    LogPanel.add('INFO', '导航', t('log.navRootAdd', { name: dragged.name }));
  } else if (dragged.source === 'nav-tree') {
    const node = popNodeByPath(State.config.nav, dragged.path);
    if (node) State.config.nav.push(node);
    LogPanel.add('INFO', '导航', t('log.navRootMoveTop', { path: dragged.path.join('.') }));
  }
  renderNavTree(); schedulePreview();
}

function handleNavDrop(dragged, targetPath, position) {
  if (dragged.source === 'page-list') {
    // 从左栏拖入：inside 或 root 都是追加到 children
    const newNode = { kind: 'page', name: dragged.name, file: dragged.file, children: [] };
    if (targetPath.length === 0) {
      State.config.nav.push(newNode);
    } else {
      const parent = getNodeByPath(State.config.nav, targetPath);
      if (parent) {
        parent.children = parent.children || [];
        parent.children.push(newNode);
      }
    }
    LogPanel.add('INFO', '导航', t('log.navPageAdd', { name: dragged.name }));
  } else {
    // 树内拖拽
    const sourceNode = popNodeByPath(State.config.nav, dragged.path);
    if (!sourceNode) return;
    // 重新计算 targetPath（如果源在目标前面）
    let tp = targetPath.slice();
    if (dragged.path.length === tp.length) {
      for (let i = 0; i < dragged.path.length - 1; i++) {
        if (dragged.path[i] !== tp[i]) break;
        if (dragged.path[dragged.path.length - 1] < tp[tp.length - 1]) {
          tp[tp.length - 1]--; // 源已移除，目标索引减 1
        }
      }
    }
    insertNodeAt(State.config.nav, tp, position, sourceNode);
    LogPanel.add('INFO', '导航', t('log.navMove', { path: dragged.path.join('.'), position, tp: tp.join('.') }));
  }
  renderNavTree(); schedulePreview();
}

function popNodeByPath(root, path) {
  if (path.length === 0) return null;
  let parent = root;
  for (let i = 0; i < path.length - 1; i++) {
    parent = parent[path[i]].children || [];
  }
  const [removed] = parent.splice(path[path.length - 1], 1);
  return removed || null;
}

function insertNodeAt(root, targetPath, position, node) {
  if (targetPath.length === 0) {
    // 根级 fallback（通常不会走到这里，根级 drop 由 handleNavRootDrop 单独处理）
    if (position === 'before') root.splice(0, 0, node);
    else root.push(node);
    return;
  }
  const parentPath = targetPath.slice(0, -1);
  const parent = parentPath.length === 0 ? root : getNodeByPath(root, parentPath).children;
  const idx = targetPath[targetPath.length - 1];

  if (position === 'before') {
    parent.splice(idx, 0, node);
  } else if (position === 'after') {
    parent.splice(idx + 1, 0, node);
  } else {
    // inside — 成为 target 的子级（追加到 children 末尾）
    const target = getNodeByPath(root, targetPath);
    if (target) {
      target.children = target.children || [];
      target.children.push(node);
    } else {
      parent.splice(idx, 0, node); // fallback
    }
  }
}

function getNodeByPath(nodes, path) {
  if (!path || path.length === 0) return null;
  let current = nodes;
  for (let i = 0; i < path.length; i++) {
    if (!current || path[i] >= current.length) return null;
    current = current[path[i]];
    if (!current) return null;
    if (i < path.length - 1) {
      current = current.children || [];
    }
  }
  return current;
}

function removeNodeByPath(nodes, path) { popNodeByPath(nodes, path); }
// 兼容旧名 — 上面已替换为 popNodeByPath

function deleteNodeByPath(path) { popNodeByPath(State.config.nav, path); }

// ========== 上下文菜单 ==========
let ctxMenuEl = null;
let ctxMenuCloseHandler = null;

function openCtxMenu(x, y, path) {
  closeCtxMenu();
  const node = getNodeByPath(State.config.nav, path);
  if (!node) return;
  const hasParent = path.length > 1;
  const isRootOnly = path.length === 1;

  const menu = document.createElement('div');
  menu.className = 'ctx-menu';
  menu.innerHTML = `
    <div class="ctx-menu-item" data-act="rename">✏️ 改名</div>
    <div class="ctx-menu-item" data-act="add-child">➕ 添加子节点</div>
    <div class="ctx-menu-sep"></div>
    <div class="ctx-menu-item ${!hasParent ? 'disabled' : ''}" data-act="outdent">⬆ 升一级</div>
    <div class="ctx-menu-item ${isRootOnly ? 'disabled' : ''}" data-act="indent">⬇ 降一级</div>
    <div class="ctx-menu-sep"></div>
    <div class="ctx-menu-item danger" data-act="delete">🗑 删除</div>
  `;

  // 定位（防止超出屏幕）
  document.body.appendChild(menu);
  const rect = menu.getBoundingClientRect();
  const maxX = window.innerWidth - rect.width - 6;
  const maxY = window.innerHeight - rect.height - 6;
  menu.style.left = Math.min(x, maxX) + 'px';
  menu.style.top = Math.min(y, maxY) + 'px';
  ctxMenuEl = menu;

  // 绑定点击
  menu.querySelectorAll('.ctx-menu-item').forEach(item => {
    if (item.classList.contains('disabled')) {
      item.style.opacity = '.35'; item.style.pointerEvents = 'none';
    }
    item.onclick = (e) => {
      e.stopPropagation();
      handleCtxMenuAct(item.dataset.act, path);
      closeCtxMenu();
    };
  });

  // 点击外部关闭
  ctxMenuCloseHandler = (e) => {
    if (!menu.contains(e.target)) closeCtxMenu();
  };
  setTimeout(() => document.addEventListener('click', ctxMenuCloseHandler), 0);
}

function closeCtxMenu() {
  if (ctxMenuEl) { ctxMenuEl.remove(); ctxMenuEl = null; }
  if (ctxMenuCloseHandler) {
    document.removeEventListener('click', ctxMenuCloseHandler);
    ctxMenuCloseHandler = null;
  }
}

function handleCtxMenuAct(act, path) {
  if (act === 'rename') {
    const node = getNodeByPath(State.config.nav, path);
    if (!node) return;
    const name = prompt(t('prompt.rename'), node.name);
    if (name && name.trim() && name.trim() !== node.name) {
      node.name = name.trim();
      renderNavTree(); schedulePreview();
    }
  }
  else if (act === 'add-child') {
    const node = getNodeByPath(State.config.nav, path);
    if (!node) return;
    node.children = node.children || [];
    const kind = prompt(t('prompt.childKind'), 'g').trim().toLowerCase();
    if (kind === 'p') {
      const name = prompt(t('prompt.childPageName'), t('default.pageName')) || t('default.pageName');
      const file = prompt(t('prompt.childFile'), '') || '';
      node.children.push({ kind: 'page', name, file, children: [] });
    } else {
      const name = prompt(t('prompt.childGroupName'), t('default.groupName')) || t('default.groupName');
      node.children.push({ kind: 'text', name, children: [] });
    }
    renderNavTree(); schedulePreview();
  }
  else if (act === 'outdent') {
    // 升一级：移动到祖父节点，成为父节点的兄弟（parent's parent children, before parent）
    if (path.length <= 1) return;
    const node = popNodeByPath(State.config.nav, path);
    if (!node) return;
    const grandParentPath = path.slice(0, -2);
    const parentIdx = path[path.length - 2];
    const grandParent = grandParentPath.length === 0
      ? State.config.nav
      : getNodeByPath(State.config.nav, grandParentPath).children;
    grandParent.splice(parentIdx + 1, 0, node);
    LogPanel.add('INFO', '导航', t('log.nodePromote', { path: path.join('.') }));
    renderNavTree(); schedulePreview();
  }
  else if (act === 'indent') {
    // 降一级：变成前一个兄弟的子节点
    if (path.length === 1 && path[0] === 0) return; // 根级第一个，没法降
    const node = popNodeByPath(State.config.nav, path);
    if (!node) return;
    const parentPath = path.slice(0, -1);
    const idx = path[path.length - 1];
    const parent = parentPath.length === 0
      ? State.config.nav
      : getNodeByPath(State.config.nav, parentPath).children;

    if (idx === 0) {
      // 没有前一个兄弟 → 变成后一个兄弟的子节点？不行，逻辑复杂。放回原位。
      parent.splice(idx, 0, node);
      showToast(t('warn.firstChildDemote'), 'warn');
      return;
    }
    const prevSibling = parent[idx - 1]; // 注意 splice 已把源弹走，所以 idx 不动
    prevSibling.children = prevSibling.children || [];
    prevSibling.children.push(node);
    LogPanel.add('INFO', '导航', t('log.nodeDemote', { path: path.join('.'), target: prevSibling.name }));
    renderNavTree(); schedulePreview();
  }
  else if (act === 'delete') {
    const node = getNodeByPath(State.config.nav, path);
    if (!node) return;
    const childCount = countNodes(node);
    const childSuffix = childCount > 1 ? t('confirm.deleteChildrenSuffix', { count: childCount - 1 }) : '';
    if (!confirm(t('confirm.deleteNodeByName', { name: node.name, children: childSuffix }))) return;
    deleteNodeByPath(path);
    State.selectedNavPath = null;
    renderNavTree(); schedulePreview();
  }
}

function countNodes(node) {
  let c = 1;
  for (const child of (node.children || [])) c += countNodes(child);
  return c;
}

// ========== 风格按钮 + 样式演示（选中后下方展开） ==========
function styleDemoText() {
  return {
    g1: t('demo.group1'), g2: t('demo.group2'),
    i1: t('demo.item1'), i2: t('demo.item2'),
    i3: t('demo.item3'), i4: t('demo.item4'),
  };
}

// 四种外壳风格的迷你演示（模拟生成外壳侧栏导航的外观，取自 templates/shell.css）
const STYLE_DEMO_BUILDERS = {
  A() {
    const d = styleDemoText();
    return `<div class="sd-a">
      <div class="sd-a-group open">
        <div class="sd-a-title"><i class="sd-arrow"></i>${escapeHtml(d.g1)}</div>
        <div class="sd-a-item active">${escapeHtml(d.i1)}</div>
        <div class="sd-a-item">${escapeHtml(d.i2)}</div>
      </div>
      <div class="sd-a-group open">
        <div class="sd-a-title"><i class="sd-arrow"></i>${escapeHtml(d.g2)}</div>
        <div class="sd-a-item">${escapeHtml(d.i3)}</div>
        <div class="sd-a-item">${escapeHtml(d.i4)}</div>
      </div>
    </div>`;
  },
  B() {
    const d = styleDemoText();
    return `<div class="sd-b">
      <div class="sd-b-title"><span>${escapeHtml(d.g1)}</span><span class="sd-b-tag">2</span></div>
      <div class="sd-b-card">
        <div class="sd-b-item active">${escapeHtml(d.i1)}</div>
        <div class="sd-b-item">${escapeHtml(d.i2)}</div>
      </div>
      <div class="sd-b-title"><span>${escapeHtml(d.g2)}</span><span class="sd-b-tag">2</span></div>
      <div class="sd-b-card">
        <div class="sd-b-item">${escapeHtml(d.i3)}</div>
        <div class="sd-b-item">${escapeHtml(d.i4)}</div>
      </div>
    </div>`;
  },
  C() {
    const d = styleDemoText();
    return `<div class="sd-c">
      <div class="sd-c-title">${escapeHtml(d.g1)}</div>
      <div class="sd-c-item active"><span class="sd-idx">1</span>${escapeHtml(d.i1)}</div>
      <div class="sd-c-item"><span class="sd-idx">2</span>${escapeHtml(d.i2)}</div>
      <div class="sd-c-title">${escapeHtml(d.g2)}</div>
      <div class="sd-c-item"><span class="sd-idx">3</span>${escapeHtml(d.i3)}</div>
      <div class="sd-c-item"><span class="sd-idx">4</span>${escapeHtml(d.i4)}</div>
    </div>`;
  },
  D() {
    const d = styleDemoText();
    return `<div class="sd-d">
      <div class="sd-d-title">${escapeHtml(d.g1)}</div>
      <div class="sd-d-item active">${escapeHtml(d.i1)}</div>
      <div class="sd-d-item">${escapeHtml(d.i2)}</div>
      <div class="sd-d-title">${escapeHtml(d.g2)}</div>
      <div class="sd-d-item">${escapeHtml(d.i3)}</div>
      <div class="sd-d-item">${escapeHtml(d.i4)}</div>
    </div>`;
  },
};

function renderStyleDemo(style) {
  const box = document.getElementById('style-demo-box');
  if (!box) return;
  const build = STYLE_DEMO_BUILDERS[style] || STYLE_DEMO_BUILDERS.C;
  box.innerHTML = build();
  box.dataset.style = style;
}

function setupStyleButtons() {
  const demoEl = document.getElementById('style-demo');
  const head = demoEl ? demoEl.querySelector('.style-demo-head') : null;
  if (head) head.onclick = () => demoEl.classList.toggle('open');
  document.querySelectorAll('.style-btn').forEach(btn => {
    btn.onclick = () => {
      document.querySelectorAll('.style-btn').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      State.config.shell_style = btn.dataset.style;
      // 选中后在下方展开该风格的样式演示
      if (demoEl) demoEl.classList.add('open');
      renderStyleDemo(btn.dataset.style);
      schedulePreview();
    };
  });
  // 初始：按当前风格渲染演示
  renderStyleDemo(State.config.shell_style);
}

// ========== 辅助文字语言（输出手册左侧展开竖条等提示语言） ==========
const AUX_LANG_NAMES = {
  'zh-CN': '简体中文', 'zh-TW': '繁體中文（台灣）', 'zh-HK': '繁體中文（香港）',
  'en': 'English', 'fr': 'Français', 'ru': 'Русский', 'es': 'Español',
  'pt': 'Português', 'ar': 'العربية', 'ja': '日本語', 'ko': '한국어',
  'el': 'Ελληνικά', 'de': 'Deutsch',
};

function renderAuxLangSelect() {
  const sel = document.getElementById('aux-lang-select');
  if (!sel) return;
  const cur = State.config.options.aux_lang || 'auto';
  const opts = [
    { v: 'auto', label: t('aux.followUi') },
    ...Object.keys(AUX_LANG_NAMES).map(v => ({ v, label: AUX_LANG_NAMES[v] })),
  ];
  sel.innerHTML = opts.map(o => `<option value="${o.v}">${escapeHtml(o.label)}</option>`).join('');
  sel.value = AUX_LANG_NAMES[cur] ? cur : 'auto';
}

function setupAuxLangSelect() {
  const sel = document.getElementById('aux-lang-select');
  if (!sel) return;
  renderAuxLangSelect();
  sel.onchange = () => {
    State.config.options.aux_lang = sel.value === 'auto' ? null : sel.value;
    LogPanel.add('INFO', '输出', t('log.auxLangSet', { lang: sel.value === 'auto' ? t('aux.followUi') : AUX_LANG_NAMES[sel.value] }));
    schedulePreview();
  };
}

// ========== 配置输入绑定 ==========
function setupConfigBindings() {
  document.getElementById('input-title').addEventListener('input', (e) => {
    State.config.title = e.target.value;
    schedulePreview();
  });
  document.getElementById('input-breadcrumb').addEventListener('input', (e) => {
    State.config.breadcrumb_prefix = e.target.value;
    schedulePreview();
  });
  document.getElementById('opt-merge-css').addEventListener('change', (e) => {
    State.config.options.merge_css = e.target.checked;
    schedulePreview();
  });
  document.getElementById('opt-collapse-sidebar').addEventListener('change', (e) => {
    State.config.options.collapsible_sidebar = e.target.checked;
    schedulePreview();
  });
}

// ========== 日志面板交互 ==========
function setupLogPanel() {
  const panel = document.getElementById('log-panel');
  const body = document.getElementById('log-body');
  const toggleBtn = document.getElementById('btn-log-toggle');
  const header = document.querySelector('.log-header');

  const toggle = () => {
    const expanded = panel.classList.toggle('expanded');
    panel.classList.toggle('collapsed', !expanded);
    body.style.display = expanded ? 'block' : 'none';
    toggleBtn.textContent = expanded ? '▼ 折叠' : '▲ 展开';
    try { localStorage.setItem('mb.log-expanded', expanded ? '1' : '0'); } catch(e) {}
  };

  header.addEventListener('click', toggle);
  toggleBtn.addEventListener('click', (e) => { e.stopPropagation(); toggle(); });

  document.getElementById('btn-log-clear').onclick = () => LogPanel.clear();
  document.getElementById('log-auto-scroll').onchange = (e) => { LogPanel.autoScroll = e.target.checked; };

  // 恢复展开偏好
  try {
    if (localStorage.getItem('mb.log-expanded') === '1') toggle();
  } catch(e) {}
}

// ========== 帮助弹窗 ==========
const HELP_DONT_SHOW_KEY = 'mb.help-dont-show';
function setupHelpModal() {
  const modal = document.getElementById('help-modal');
  if (!modal) return;
  const dontShow = document.getElementById('help-dont-show');
  const readPref = () => {
    try { return localStorage.getItem(HELP_DONT_SHOW_KEY) === '1'; } catch (e) { return false; }
  };
  const savePref = (v) => {
    try { localStorage.setItem(HELP_DONT_SHOW_KEY, v ? '1' : '0'); } catch (e) {}
  };
  const open = () => {
    if (dontShow) dontShow.checked = readPref();
    modal.style.display = 'flex';
  };
  const close = () => {
    if (dontShow && dontShow.checked) savePref(true);
    modal.style.display = 'none';
  };
  const btn = document.getElementById('btn-help');
  const btnClose = document.getElementById('btn-help-close');
  if (btn) btn.onclick = open;
  if (btnClose) btnClose.onclick = close;
  if (dontShow) dontShow.onchange = (e) => savePref(e.target.checked);
  modal.addEventListener('click', (e) => { if (e.target === modal) close(); });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && modal.style.display !== 'none') close();
  });
  // 首次启动（或未勾选「不再显示」时）自动弹出；延后到 init 完成后显示
  if (!readPref()) setTimeout(open, 0);
}

// ========== 体积超限导出确认弹窗 ==========
let splitConfirmResolve = null;

// ≥20MB 导出前询问：返回 Promise<boolean>（true=继续导出，false=取消）
function askSplitConfirm() {
  const modal = document.getElementById('split-confirm-modal');
  const msg = document.getElementById('split-confirm-msg');
  const sizeStr = State.lastStats ? formatSize(State.lastStats.final_bytes) : '';
  // i18n 文案内含 <span class="confirm-size">{size}</span>，故用 innerHTML
  msg.innerHTML = t('confirm.splitMsg', { size: sizeStr });
  modal.style.display = 'flex';
  return new Promise((resolve) => { splitConfirmResolve = resolve; });
}

function settleSplitConfirm(result) {
  document.getElementById('split-confirm-modal').style.display = 'none';
  if (splitConfirmResolve) {
    splitConfirmResolve(result);
    splitConfirmResolve = null;
  }
}

function setupSplitConfirm() {
  document.getElementById('btn-split-continue').onclick = () => settleSplitConfirm(true);
  document.getElementById('btn-split-cancel').onclick = () => settleSplitConfirm(false);
}

// ========== 缺失引用资源补齐 ==========
// 场景：只拖入单个 HTML 时，其引用的同目录图片/CSS/JS 不在拖入内容中。
// 打包端收集缺失引用（url + 期望路径），此处先自动搜索补齐（无弹窗）：
//   ① 导入目录/输入目录/历史补齐目录（页面"同级"及记忆位置）
//   ② 系统常用目录（桌面/文档/下载/图片，首次导入也能命中）
// 仍有缺失时才降级为手动选择文件夹弹窗。
let missingPromptKey = null; // 已提示过（含已忽略）的缺失集合签名，避免重复弹窗

function getRememberedRepairDirs() {
  try { return JSON.parse(localStorage.getItem('mb.repairDirs') || '[]'); } catch (e) { return []; }
}

function rememberRepairDir(dir) {
  if (!dir) return;
  try {
    let dirs = getRememberedRepairDirs().filter(d => d !== dir);
    dirs.unshift(dir);
    dirs = dirs.slice(0, 5);
    localStorage.setItem('mb.repairDirs', JSON.stringify(dirs));
  } catch (e) {}
}

// 自动补齐：返回仍未解决的缺失列表（null = 浏览器降级模式）
async function autoRepairMissing(missing) {
  const extraDirs = [...new Set([...getRememberedRepairDirs(), ...State.config.input_dirs])];
  const result = await invoke('auto_repair_missing_resources', { missing, extraDirs });
  if (result === null) return null;
  if (result.repaired > 0) {
    LogPanel.add('INFO', '资源补齐', t('log.autoRepairDone', { count: result.repaired, dir: result.repaired_from[0] || '' }));
    showToast(t('toast.autoRepairDone', { count: result.repaired }), 'info');
    (result.repaired_from || []).forEach(rememberRepairDir);
  }
  return result.still_missing;
}

async function handleMissingResources(missing) {
  if (!missing || missing.length === 0) { missingPromptKey = null; return; }
  const key = missing.map(m => m.attempted).sort().join('|');
  if (key === missingPromptKey) return; // 同一批缺失已处理过（弹窗/忽略中）
  missingPromptKey = key;
  // 第一步：自动搜索补齐（无弹窗）
  let still = missing;
  try {
    still = await autoRepairMissing(missing);
  } catch (e) {
    LogPanel.add('WARN', '资源补齐', t('log.repairFail', { err: e.message || e }));
  }
  if (still === null) return; // 浏览器降级模式
  if (still.length > 0) {
    // 第二步：自动搜索未命中的，降级为手动选择文件夹
    showMissingModal(still);
  } else {
    schedulePreview(); // 全部自动补齐 → 重新打包内嵌
  }
}

function showMissingModal(still) {
  const modal = document.getElementById('missing-modal');
  const list = document.getElementById('missing-list');
  list.innerHTML = still.slice(0, 6).map(m =>
    `<div class="missing-item" title="${escapeAttr(m.url)}">${escapeHtml(m.url)}</div>`
  ).join('') + (still.length > 6
    ? `<div class="missing-item" style="color:var(--sub)">… +${still.length - 6}</div>` : '');
  modal.style.display = 'flex';
  State.pendingMissing = still;
}

function closeMissingModal() {
  document.getElementById('missing-modal').style.display = 'none';
}

async function onRepairMissing() {
  const missing = State.pendingMissing || [];
  closeMissingModal();
  if (!missing.length) return;
  try {
    const folder = await open({ directory: true, multiple: false });
    if (!folder) { missingPromptKey = null; schedulePreview(); return; }
    const repaired = await invoke('repair_missing_resources', { folder, missing });
    if (repaired === null) { showToast(t('toast.mockBundle'), 'warn'); return; }
    if (repaired > 0) {
      LogPanel.add('INFO', '资源补齐', t('log.repairDone', { count: repaired }));
      showToast(t('toast.repairDone', { count: repaired }), 'info');
      rememberRepairDir(folder); // 记住该目录，下次自动搜索优先使用
      missingPromptKey = null;
      schedulePreview(); // 重新打包，让补齐后的资源完成内嵌
    } else {
      LogPanel.add('WARN', '资源补齐', t('log.repairDone', { count: 0 }));
      showToast(t('toast.repairNone'), 'warn');
    }
  } catch (e) {
    LogPanel.add('ERROR', '资源补齐', t('log.repairFail', { err: e.message || e }));
    showToast(t('log.repairFail', { err: e.message || e }), 'error');
  }
}

function setupMissingModal() {
  document.getElementById('btn-missing-pick').onclick = onRepairMissing;
  document.getElementById('btn-missing-ignore').onclick = () => closeMissingModal();
}

// ========== OS 级拖拽导入 ==========
// 说明：新版 WebView2 Runtime 已移除 DOM 侧非标准的 File.path，
// 无法再从 drop 事件直接拿到绝对路径。因此改为：
//   webkitGetAsEntry 递归读取拖入的文件/文件夹内容（base64）
//   → invoke('import_dropped_files') 由 Rust 落盘到临时导入目录
//   → addInputDir(导入目录) 走常规扫描流程。
// 内部拖拽（页面列表 ↔ 导航树）仍由 HTML5 DnD 处理，不受影响。
function setupOsDragDrop() {
  const app = document.getElementById('app');

  // 与 Rust is_page_ext 对齐的页面扩展名判定
  const PAGE_EXTS = ['html', 'htm', 'mht', 'mhtml', 'png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg', 'ico', 'avif',
    'pdf', 'doc', 'docx', 'rtf', 'wps', 'odt', 'odp', 'ppt', 'pptx', 'txt', 'md'];
  const isPageFile = (p) => {
    const m = String(p).toLowerCase().match(/\.([a-z0-9]+)$/);
    return !!m && PAGE_EXTS.includes(m[1]);
  };

  function bufToBase64(buf) {
    const bytes = new Uint8Array(buf);
    let bin = '';
    const CHUNK = 0x8000;
    for (let i = 0; i < bytes.length; i += CHUNK) {
      bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
    }
    return btoa(bin);
  }

  // 收集拖入根 entry —— 必须在 drop 事件内同步调用（事件结束后 dataTransfer 即失效）
  function collectRootEntries(dataTransfer) {
    const entries = [];
    for (const it of Array.from(dataTransfer.items || [])) {
      const entry = it.webkitGetAsEntry && it.webkitGetAsEntry();
      if (entry) entries.push(entry);
    }
    if (entries.length === 0) {
      // 兜底：无 entry 时退回 File 列表（相对路径只有文件名）
      for (const f of Array.from(dataTransfer.files || [])) {
        entries.push({ isFile: true, isDirectory: false, name: f.name, fullPath: '/' + f.name, __file: f });
      }
    }
    return entries;
  }

  // 递归展开 entry → [{ relPath, file }]
  function walkEntry(entry, out) {
    return new Promise((resolve) => {
      if (entry.isFile) {
        if (entry.__file) {
          out.push({ relPath: entry.fullPath.replace(/^\//, ''), file: entry.__file });
          return resolve();
        }
        entry.file((f) => {
          out.push({ relPath: (entry.fullPath || '/' + f.name).replace(/^\//, ''), file: f });
          resolve();
        }, () => resolve());
      } else if (entry.isDirectory) {
        const reader = entry.createReader();
        const kids = [];
        // readEntries 每次最多返回约 100 条，须循环读到空为止
        const step = () => reader.readEntries((res) => {
          if (!res.length) {
            Promise.all(kids.map((k) => walkEntry(k, out))).then(() => resolve(), () => resolve());
          } else {
            kids.push(...res);
            step();
          }
        }, () => resolve());
        step();
      } else {
        resolve();
      }
    });
  }

  app.addEventListener('dragover', (e) => {
    if (State.draggedItem) return;
    if (e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files')) {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
    }
  });

  app.addEventListener('drop', async (e) => {
    if (State.draggedItem) return;
    const hasFiles = Array.from(e.dataTransfer?.types || []).includes('Files');
    if (!hasFiles) return;
    e.preventDefault();

    // 同步收集 entry，再异步展开
    const rootEntries = collectRootEntries(e.dataTransfer);
    const collected = [];
    for (const entry of rootEntries) {
      await walkEntry(entry, collected);
    }
    const pageFiles = collected.filter((c) => isPageFile(c.relPath));
    if (collected.length === 0 || pageFiles.length === 0) {
      LogPanel.add('WARN', '前端', t('log.osDropNoPage', { count: collected.length }));
      showToast(t('toast.osDropNoPage'), 'warn');
      return;
    }

    // 全量读取（页面 + 其引用的图片/CSS/JS 等资源）→ base64，保证页面外链资源内嵌不缺失
    const payload = [];
    let readFailed = 0;
    for (const c of collected) {
      try {
        const buf = await c.file.arrayBuffer();
        payload.push({ relPath: c.relPath, dataBase64: bufToBase64(buf) });
      } catch (err) {
        readFailed++;
        LogPanel.add('WARN', '前端', t('log.osDropFail', { path: c.relPath, err }));
      }
    }
    if (payload.length === 0) {
      showToast(t('toast.osDropNoPage'), 'warn');
      return;
    }

    try {
      const result = await invoke('import_dropped_files', { files: payload });
      if (!result) {
        showToast(t('toast.mockBundle'), 'warn');
        return;
      }
      LogPanel.add('INFO', '前端', t('log.importDone', { count: result.saved.length, dir: result.dir }));
      const skippedTotal = result.skipped + readFailed;
      if (skippedTotal > 0) {
        LogPanel.add('WARN', '前端', t('log.importSkipped', { count: skippedTotal }));
      }
      await addInputDir(result.dir);
    } catch (err) {
      LogPanel.add('ERROR', '前端', t('log.osDropFail', { path: '(导入)', err }));
      showToast(t('toast.importFail', { err: err.message || err }), 'error');
    }
  });
}

// ========== 启动 ==========
async function init() {
  LogPanel.add('INFO', '启动', t('log.initDone', { tauri: tauriAvailable ? '是' : '否' }));

  // [诊断] 打印 __TAURI__ 实际暴露的命名空间
  if (tauriAvailable) {
    try {
      const keys = Object.keys(__TAURI__).sort();
      LogPanel.add('INFO', '启动', `__TAURI__ keys: ${keys.join(', ')}`);
      // 深入各子对象
      for (const k of keys) {
        const sub = __TAURI__[k];
        if (sub && typeof sub === 'object') {
          const subKeys = Object.getOwnPropertyNames(sub).slice(0, 15).join(',');
          LogPanel.add('DEBUG', '启动', `  __TAURI__.${k}: [${subKeys}]`);
        }
      }
    } catch(e) { LogPanel.add('WARN', '启动', `__TAURI__ 诊断失败: ${e}`); }
  }

  try {
    await setupListeners();
  } catch (e) {
    console.warn('Event setup failed:', e);
  }

  if (window.I18N) {
    try { window.I18N.init(); } catch(e) { console.warn('i18n init failed:', e); }
  }

  setupStyleButtons();
  setupConfigBindings();
  setupAuxLangSelect();
  setupLogPanel();
  setupHelpModal();
  setupSplitConfirm();
  setupMissingModal();

  document.getElementById('btn-open-output').onclick = onOpenOutputFolder;
  document.getElementById('btn-generate').onclick = onGenerate;
  document.getElementById('btn-add-dir').onclick = onOpenFolder;
  document.getElementById('btn-add-group').onclick = onAddGroup;
  document.getElementById('btn-select-output').onclick = onSelectOutput;

  // 语言切换：切换后重渲染所有动态 UI（静态区由 I18N.reapply 处理）；
  // locale 同步写入配置，使「辅助文字语言=跟随界面语言」时输出手册随之转换
  const langSelect = document.getElementById('lang-select');
  if (langSelect) {
    langSelect.value = window.I18N ? I18N.locale : 'zh-CN';
    langSelect.onchange = () => {
      I18N.loadLocale(langSelect.value);
      State.config.locale = langSelect.value;
      renderInputDirs();
      renderOutputPath();
      renderPageList();
      renderNavTree();
      renderStyleDemo(State.config.shell_style);
      renderAuxLangSelect();
      schedulePreview();
    };
  }

  try { setupOsDragDrop(); } catch(e) { console.warn(e); }

  // 初始渲染（含空输入时的右栏实时预览占位）
  State.config.locale = window.I18N ? I18N.locale : 'zh-CN';
  renderInputDirs();
  renderOutputPath();
  renderPageList();
  renderNavTree();
  doPreview();
}

init();
