(function () {
  const API = () => window.AMBIENTOR_API_URL || '';
  const TOKEN_KEY = 'ambientor_token';

  const $ = (id) => document.getElementById(id);

  let authConfig = {
    enabled: false,
    localLogin: false,
    oidcLoginUrl: null,
    requireAuthForApprove: false,
  };

  let applicationsPage = { items: [], total: 0, page: 1, pageSize: 50 };
  let appListPage = 1;
  let appListFilters = { q: '', riskLevel: '', meshRevision: '' };
  let selectedAppNamespace = null;
  let plans = [];
  let selectedPlanKey = null;
  let rollouts = [];
  let selectedRolloutKey = null;
  let rolloutDetail = null;

  function showPanel(id) {
    document.querySelectorAll('main .panel').forEach((p) => p.classList.add('hidden'));
    const panel = document.getElementById(id);
    if (panel) panel.classList.remove('hidden');
    document.querySelectorAll('nav a').forEach((a) => {
      a.classList.toggle('active', a.getAttribute('href') === '#' + id);
    });
  }

  function setStatus(msg, isError) {
    const el = $('status-banner');
    if (!el) return;
    el.textContent = msg || '';
    el.className = 'status-banner' + (msg ? (isError ? ' error' : ' info') : ' hidden');
  }

  function getToken() {
    return localStorage.getItem(TOKEN_KEY);
  }

  function setToken(token) {
    if (token) localStorage.setItem(TOKEN_KEY, token);
    else localStorage.removeItem(TOKEN_KEY);
  }

  function parseJwtUsername(token) {
    try {
      const payload = token.split('.')[1];
      const padded = payload.replace(/-/g, '+').replace(/_/g, '/');
      const json = JSON.parse(atob(padded));
      return json.sub || json.username || null;
    } catch {
      return null;
    }
  }

  function authHeaders(extra) {
    const h = Object.assign({}, extra || {});
    const t = getToken();
    if (t) h.Authorization = 'Bearer ' + t;
    return h;
  }

  function consumeOidcTokenFromUrl() {
    const params = new URLSearchParams(window.location.search);
    const token = params.get('token');
    if (!token) return;
    setToken(token);
    params.delete('token');
    const qs = params.toString();
    const path = window.location.pathname + window.location.hash;
    window.history.replaceState({}, '', path + (qs ? '?' + qs : ''));
  }

  function canApproveRollout(awaiting) {
    if (!awaiting) return false;
    if (authConfig.requireAuthForApprove && !getToken()) return false;
    return true;
  }

  function updateApproveAuthHint() {
    const hint = $('approve-auth-hint');
    if (!hint) return;
    const needsLogin = authConfig.requireAuthForApprove && !getToken();
    hint.classList.toggle('hidden', !needsLogin);
    if (needsLogin) {
      hint.textContent =
        'Sign in (local or SSO) to approve rollout stages when the API has auth enabled.';
    }
  }

  function updateAuthUi() {
    const loggedIn = !!getToken();
    const authOn = authConfig.enabled;

    $('auth-disabled-hint')?.classList.toggle('hidden', authOn);
    $('auth-login-panel')?.classList.toggle('hidden', !authOn || loggedIn);
    $('auth-user-panel')?.classList.toggle('hidden', !loggedIn);

    if (loggedIn && $('auth-username')) {
      $('auth-username').textContent = parseJwtUsername(getToken()) || 'user';
    }

    const oidcBtn = $('auth-oidc-login');
    if (oidcBtn) {
      const showOidc = authOn && !!authConfig.oidcLoginUrl && !loggedIn;
      oidcBtn.classList.toggle('hidden', !showOidc);
    }

    $('auth-local-form')?.classList.toggle('hidden', !authConfig.localLogin || loggedIn);
    updateApproveAuthHint();

    const r = rollouts.find((x) => rolloutKey(x) === selectedRolloutKey);
    if (r) {
      const awaiting = r.awaitingApproval || r.awaiting_approval;
      $('approve-rollout').disabled = !canApproveRollout(awaiting);
    }
  }

  async function loadAuthConfig() {
    try {
      const res = await fetch(API() + '/api/v1/auth/config');
      if (!res.ok) throw new Error(await res.text());
      authConfig = await res.json();
    } catch {
      authConfig = {
        enabled: false,
        localLogin: false,
        oidcLoginUrl: null,
        requireAuthForApprove: false,
      };
    }
    updateAuthUi();
  }

  async function loginLocal() {
    const user = $('auth-username-input')?.value?.trim();
    const pass = $('auth-password-input')?.value;
    if (!user || !pass) {
      setStatus('Enter username and password', true);
      return;
    }
    setStatus('Signing in…');
    try {
      const res = await fetch(API() + '/api/v1/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username: user, password: pass }),
      });
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      setToken(data.token);
      if ($('auth-password-input')) $('auth-password-input').value = '';
      updateAuthUi();
      setStatus('Signed in as ' + (parseJwtUsername(data.token) || user));
    } catch (e) {
      setStatus('Login failed: ' + e.message, true);
    }
  }

  function logout() {
    setToken(null);
    updateAuthUi();
    setStatus('Signed out');
  }

  function startOidcLogin() {
    const path = authConfig.oidcLoginUrl || '/api/v1/auth/oidc/login';
    window.location.href = API() + path;
  }

  function renderScores(scores, prefix) {
    const overall = $(prefix + 'overall-score');
    if (overall) overall.textContent = scores?.overall ?? '—';
    const readiness = $(prefix + 'readiness');
    const sidecar = $(prefix + 'sidecar');
    const traffic = $(prefix + 'traffic');
    if (readiness) readiness.textContent = scores?.readiness ?? '—';
    if (sidecar) {
      sidecar.textContent = scores?.sidecarDependency ?? scores?.sidecar_dependency ?? '—';
    }
    if (traffic) {
      traffic.textContent = scores?.trafficCompatibility ?? scores?.traffic_compatibility ?? '—';
    }
  }

  function renderSummary(summary, prefix) {
    $(prefix + 'blockers').textContent = summary?.blockers ?? 0;
    $(prefix + 'warnings').textContent = summary?.warnings ?? 0;
    const info = $(prefix + 'info');
    if (info) info.textContent = summary?.info ?? 0;
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function renderFinding(f) {
    const li = document.createElement('li');
    li.className = 'finding ' + (f.severity || 'info');
    const evidence = f.evidence
      ? `<pre class="evidence">${escapeHtml(f.evidence)}</pre>`
      : '';
    const resource = f.resource ? `<span class="meta">${escapeHtml(f.resource)}</span>` : '';
    const ns = f.namespace ? `<span class="meta">ns: ${escapeHtml(f.namespace)}</span>` : '';
    const docUrl = f.docUrl || f.doc_url;
    li.innerHTML = `
      <div class="finding-head">
        <span class="badge ${f.severity}">${escapeHtml(f.severity)}</span>
        <strong>${escapeHtml(f.title)}</strong>
        ${resource}${ns}
      </div>
      <p class="message">${escapeHtml(f.message)}</p>
      ${evidence}
      ${f.remediation ? `<p class="remediation"><strong>Remediation:</strong> ${escapeHtml(f.remediation)}</p>` : ''}
      ${docUrl ? `<a class="doc-link" href="${escapeHtml(docUrl)}" target="_blank" rel="noopener">Documentation</a>` : ''}
    `;
    return li;
  }

  function renderFindings(findings, listId) {
    const list = $(listId);
    if (!list) return;
    list.innerHTML = '';
    (findings || []).forEach((f) => list.appendChild(renderFinding(f)));
  }

  function riskBadgeClass(risk) {
    const r = (risk || '').toLowerCase();
    return 'risk-badge ' + (r || 'low');
  }

  function formatLabels(labels) {
    if (!labels || typeof labels !== 'object') return '—';
    const entries = Object.entries(labels);
    if (!entries.length) return '—';
    return entries.map(([k, v]) => `${k}=${v}`).join(', ');
  }

  function formatDataplane(app) {
    const mode = app.dataplaneMode || app.dataplane_mode;
    if (mode === 'ambient' || mode === 'sidecar' || mode === 'notEnrolled') {
      if (mode === 'notEnrolled') return 'not enrolled';
      return mode;
    }
    if (app.ambientDataplane || app.ambient_dataplane) return 'ambient';
    const labels = app.namespaceLabels || app.namespace_labels;
    if (labels && labels['istio.io/dataplane-mode'] === 'ambient') return 'ambient';
    if (
      labels &&
      (labels['istio.io/rev'] ||
        labels['istio-discovery'] ||
        labels['istio-injection'] === 'enabled' ||
        labels['istio-injection'] === 'true')
    ) {
      return 'sidecar';
    }
    return '—';
  }

  function dataplaneBadgeClass(mode) {
    const m = (mode || '').toLowerCase();
    if (m === 'ambient') return 'dataplane-ambient';
    if (m === 'sidecar') return 'dataplane-sidecar';
    return 'dataplane-unknown';
  }

  function formatIngress(app) {
    if (!app.ingressGatewayNamespace && !app.ingress_gateway_namespace) return '—';
    if (app.ingressSameNamespace || app.ingress_same_namespace) return 'Same namespace';
    return `Separate (${app.ingressGatewayNamespace || app.ingress_gateway_namespace})`;
  }

  function formatHostnames(hostnames) {
    if (!hostnames || !hostnames.length) return '—';
    if (hostnames.length <= 2) return hostnames.join(', ');
    return `${hostnames.slice(0, 2).join(', ')} +${hostnames.length - 2}`;
  }

  function formatControlPlane(app) {
    const parts = [
      app.discoveryLabel || app.discovery_label,
      app.meshRevision || app.mesh_revision,
    ].filter(Boolean);
    return parts.length ? parts.join(' · ') : '—';
  }

  function renderMeshFilterOptions() {
    const select = $('app-mesh-filter');
    if (!select) return;
    const revisions = new Set();
    (applicationsPage.items || []).forEach((a) => {
      const rev = a.meshRevision || a.mesh_revision;
      if (rev) revisions.add(rev);
    });
    const current = select.value;
    select.innerHTML = '<option value="">All control planes</option>';
    [...revisions].sort().forEach((rev) => {
      const opt = document.createElement('option');
      opt.value = rev;
      opt.textContent = rev;
      select.appendChild(opt);
    });
    select.value = current;
  }

  function renderApplicationsTable() {
    const tbody = $('app-assess-tbody');
    if (!tbody) return;
    const items = applicationsPage.items || [];
    if (!items.length) {
      tbody.innerHTML =
        '<tr><td colspan="8" class="hint">No applications match filters. Run assessment to scan the cluster.</td></tr>';
      return;
    }
    tbody.innerHTML = items
      .map((app) => {
        const ns = app.namespace;
        const selected = ns === selectedAppNamespace ? ' selected' : '';
        const readiness = app.readinessPct ?? app.readiness_pct ?? 0;
        const risk = app.riskLevel || app.risk_level || 'low';
        const dp = formatDataplane(app);
        return `<tr class="app-row${selected}" data-ns="${escapeHtml(ns)}" tabindex="0">
          <td><strong>${escapeHtml(ns)}</strong></td>
          <td>${escapeHtml(formatControlPlane(app))}</td>
          <td><span class="badge-dataplane ${dataplaneBadgeClass(dp)}">${escapeHtml(dp)}</span></td>
          <td class="mono">${escapeHtml(formatHostnames(app.hostnames))}</td>
          <td class="mono small">${escapeHtml(formatLabels(app.namespaceLabels || app.namespace_labels))}</td>
          <td>${escapeHtml(formatIngress(app))}</td>
          <td>
            <div class="readiness-cell">
              <div class="readiness-bar"><span style="width:${readiness}%"></span></div>
              <span>${readiness}%</span>
            </div>
          </td>
          <td><span class="${riskBadgeClass(risk)}">${escapeHtml(String(risk))}</span></td>
        </tr>`;
      })
      .join('');

    tbody.querySelectorAll('.app-row').forEach((row) => {
      const ns = row.getAttribute('data-ns');
      row.addEventListener('click', () => openApplicationDetail(ns));
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          openApplicationDetail(ns);
        }
      });
    });
  }

  function updatePaginationUi() {
    const total = applicationsPage.total || 0;
    const page = applicationsPage.page || appListPage;
    const pageSize = applicationsPage.pageSize || applicationsPage.page_size || 50;
    const pages = Math.max(1, Math.ceil(total / pageSize));
    const info = $('app-page-info');
    if (info) {
      info.textContent = total
        ? `Page ${page} of ${pages} · ${total.toLocaleString()} application(s)`
        : 'No applications';
    }
    const prev = $('app-page-prev');
    const next = $('app-page-next');
    if (prev) prev.disabled = page <= 1;
    if (next) next.disabled = page >= pages;
  }

  function applicationsQueryString() {
    const params = new URLSearchParams();
    params.set('page', String(appListPage));
    params.set('pageSize', '50');
    if (appListFilters.q) params.set('q', appListFilters.q);
    if (appListFilters.riskLevel) params.set('riskLevel', appListFilters.riskLevel);
    if (appListFilters.meshRevision) params.set('meshRevision', appListFilters.meshRevision);
    return params.toString();
  }

  async function loadApplications() {
    setStatus('Loading applications…');
    try {
      const res = await fetch(API() + '/api/v1/applications?' + applicationsQueryString());
      if (res.status === 503) {
        throw new Error('Database not configured (set DATABASE_URL on API)');
      }
      if (!res.ok) throw new Error(await res.text());
      applicationsPage = await res.json();
      appListPage = applicationsPage.page || 1;
      const meta = $('assess-meta');
      if (meta) {
        const when = applicationsPage.lastAssessedAt || applicationsPage.last_assessed_at;
        meta.textContent = when
          ? `${applicationsPage.total.toLocaleString()} application(s) · last assessed ${new Date(when).toLocaleString()}`
          : 'Run assessment to populate the application catalog in the database.';
      }
      renderMeshFilterOptions();
      renderApplicationsTable();
      updatePaginationUi();
      setStatus(`Loaded ${applicationsPage.total.toLocaleString()} application(s)`);
    } catch (e) {
      setStatus('Failed to load applications: ' + e.message, true);
    }
  }

  function closeApplicationDetail() {
    const drawer = $('app-detail-drawer');
    if (drawer) {
      drawer.classList.add('hidden');
      drawer.setAttribute('aria-hidden', 'true');
    }
    selectedAppNamespace = null;
    renderApplicationsTable();
  }

  async function openApplicationDetail(namespace) {
    selectedAppNamespace = namespace;
    renderApplicationsTable();
    const drawer = $('app-detail-drawer');
    if (drawer) {
      drawer.classList.remove('hidden');
      drawer.setAttribute('aria-hidden', 'false');
    }
    $('app-detail-title').textContent = namespace;
    $('app-detail-meta').innerHTML = '<p class="hint">Loading…</p>';
    setStatus('Loading application detail…');
    try {
      const res = await fetch(
        API() + '/api/v1/applications/' + encodeURIComponent(namespace)
      );
      if (!res.ok) throw new Error(await res.text());
      const detail = await res.json();
      const app = detail.list || detail;
      $('app-detail-meta').innerHTML = `
        <dl class="meta-dl">
          <dt>Control plane</dt><dd>${escapeHtml(formatControlPlane(app))}</dd>
          <dt>Revision NS</dt><dd>${escapeHtml(app.controlPlaneNamespace || app.control_plane_namespace || '—')}</dd>
          <dt>Hostnames</dt><dd class="mono">${escapeHtml((app.hostnames || []).join(', ') || '—')}</dd>
          <dt>Dataplane</dt><dd><span class="badge-dataplane ${dataplaneBadgeClass(formatDataplane(app))}">${escapeHtml(formatDataplane(app))}</span></dd>
          <dt>Istio labels</dt><dd class="mono small">${escapeHtml(formatLabels(app.namespaceLabels || app.namespace_labels))}</dd>
          <dt>Ingress gateway</dt><dd>${escapeHtml(formatIngress(app))}</dd>
          <dt>Workloads</dt><dd>${app.workloadCount ?? app.workload_count ?? 0}</dd>
        </dl>
      `;
      renderScores(detail.scores, 'detail-');
      renderSummary(detail.summary, 'detail-');
      renderFindings(detail.findings, 'detail-findings');
      const sugUl = $('app-detail-suggestions');
      if (sugUl) {
        sugUl.innerHTML = '';
        (detail.suggestions || []).forEach((s) => {
          const li = document.createElement('li');
          li.className = 'suggestion-card';
          li.innerHTML = `
            <span class="badge-status ${escapeHtml(s.severity)}">${escapeHtml(s.severity)}</span>
            <strong>${escapeHtml(s.title)}</strong>
            <p>${escapeHtml(s.remediation)}</p>
          `;
          sugUl.appendChild(li);
        });
        if (!(detail.suggestions || []).length) {
          sugUl.innerHTML = '<li class="hint">No remediation suggestions for this application.</li>';
        }
      }
      setStatus(`Application ${namespace} loaded`);
    } catch (e) {
      setStatus('Failed to load detail: ' + e.message, true);
    }
  }

  async function loadAssessments() {
    await loadApplications();
  }

  function statusLabel(status) {
    const map = {
      migrated: 'Migrated',
      processing: 'Processing',
      blocker: 'Blocker',
      failed: 'Failed',
      scanned: 'Scanned',
      notScanned: 'Not scanned',
    };
    return map[status] || status;
  }

  function statusCssClass(status) {
    if (status === 'notScanned') return 'not-scanned';
    return status;
  }

  function renderStatusSummary(counts) {
    const c = counts || {};
    $('sum-migrated').textContent = c.migrated ?? 0;
    $('sum-processing').textContent = c.processing ?? 0;
    $('sum-blocker').textContent = c.blocker ?? 0;
    $('sum-failed').textContent = c.failed ?? 0;
    $('sum-scanned').textContent = c.scanned ?? 0;
    $('sum-not-scanned').textContent = c.notScanned ?? c.not_scanned ?? 0;
  }

  function renderIstiodCard(mesh) {
    const card = document.createElement('article');
    card.className = 'istiod-card ' + (mesh.ambient ? 'ambient' : 'sidecar');
    const counts = mesh.counts || {};
    const pills = [];
    if (counts.migrated) pills.push(`<span class="pill migrated">${counts.migrated} migrated</span>`);
    if (counts.processing) pills.push(`<span class="pill processing">${counts.processing} processing</span>`);
    if (counts.blocker) pills.push(`<span class="pill blocker">${counts.blocker} blocker</span>`);
    if (counts.failed) pills.push(`<span class="pill failed">${counts.failed} failed</span>`);
    if (counts.scanned) pills.push(`<span class="pill scanned">${counts.scanned} scanned</span>`);
    if (counts.notScanned || counts.not_scanned) {
      const n = counts.notScanned ?? counts.not_scanned;
      pills.push(`<span class="pill not-scanned">${n} not scanned</span>`);
    }

    const rows = (mesh.applications || [])
      .map((app) => {
        const st = app.status || 'notScanned';
        const dp = formatDataplane(app);
        const assess = app.assessmentRef || app.assessment_ref || '—';
        return `<tr>
          <td><strong>${escapeHtml(app.namespace)}</strong></td>
          <td><span class="badge-status ${statusCssClass(st)}">${escapeHtml(statusLabel(st))}</span></td>
          <td><span class="badge-dataplane ${dataplaneBadgeClass(dp)}">${escapeHtml(dp)}</span></td>
          <td>${escapeHtml(assess)}</td>
        </tr>`;
      })
      .join('');

    const kind = mesh.ambient ? 'ambient' : 'sidecar';
    card.innerHTML = `
      <div class="istiod-card-head">
        <div>
          <h4>${escapeHtml(mesh.discoveryLabel || mesh.discovery_label)}</h4>
          <p class="istiod-sub">revision <code>${escapeHtml(mesh.revision)}</code> · ns <code>${escapeHtml(mesh.controlPlaneNamespace || mesh.control_plane_namespace)}</code> · ${kind}</p>
        </div>
        <div class="istiod-counts">${pills.join('') || '<span class="pill">No applications</span>'}</div>
      </div>
      <table class="app-table">
        <thead><tr><th>Application (namespace)</th><th>Status</th><th>Dataplane</th><th>Assessment</th></tr></thead>
        <tbody>${rows || '<tr><td colspan="4">No enrolled namespaces on this control plane</td></tr>'}</tbody>
      </table>
    `;
    return card;
  }

  async function loadDashboard() {
    setStatus('Loading dashboard…');
    const container = $('dash-mesh-instances');
    try {
      const res = await fetch(API() + '/api/v1/dashboard?fresh=true');
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      const cluster = data.cluster || {};
      $('dash-cluster-name').textContent = cluster.name || 'Connected cluster';
      const meta = [
        cluster.platform,
        cluster.meshFlavor || cluster.mesh_flavor,
        cluster.istioVersion || cluster.istio_version
          ? 'Istio ' + (cluster.istioVersion || cluster.istio_version)
          : null,
        (cluster.meshInstanceCount ?? cluster.mesh_instance_count ?? 0) +
          ' control plane(s)',
      ]
        .filter(Boolean)
        .join(' · ');
      $('dash-cluster-meta').textContent = meta || '—';
      if (data.lastUpdated || data.last_updated) {
        $('dash-last-updated').textContent =
          'Updated ' + new Date(data.lastUpdated || data.last_updated).toLocaleString();
      }
      renderStatusSummary(data.summary);
      if (container) {
        container.innerHTML = '';
        (data.meshInstances || data.mesh_instances || []).forEach((m) => {
          container.appendChild(renderIstiodCard(m));
        });
        if (!(data.meshInstances || data.mesh_instances || []).length) {
          container.innerHTML = '<p class="hint">No Istio control planes discovered.</p>';
        }
      }
      setStatus('Dashboard loaded');
    } catch (e) {
      if (container) container.innerHTML = '';
      setStatus('Dashboard failed: ' + e.message, true);
    }
  }

  async function runAssessment() {
    setStatus('Running assessment…');
    const btn = $('run-assess');
    if (btn) btn.disabled = true;
    try {
      const res = await fetch(API() + '/api/v1/assess', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      const count = data.applicationCount ?? data.application_count ?? 0;
      setStatus(
        `Assessment complete — ${count.toLocaleString()} application(s) updated in database`
      );
      showPanel('assessments');
      appListPage = 1;
      closeApplicationDetail();
      await loadApplications();
      await loadDashboard();
    } catch (e) {
      setStatus('Assessment failed: ' + e.message, true);
    } finally {
      if (btn) btn.disabled = false;
    }
  }

  function appendEvent(data) {
    const el = $('live-events');
    const p = document.createElement('p');
    try {
      const parsed = JSON.parse(data);
      p.textContent = `[${parsed.channel || 'event'}] ${JSON.stringify(parsed.payload || parsed)}`;
    } catch {
      p.textContent = data;
    }
    el.prepend(p);
    while (el.children.length > 50) el.removeChild(el.lastChild);
  }

  function planKey(p) {
    return p.namespace + '/' + p.name;
  }

  function renderPlanList() {
    const ul = $('plan-list');
    if (!ul) return;
    ul.innerHTML = '';
    plans.forEach((p) => {
      const li = document.createElement('li');
      const key = planKey(p);
      li.className = 'assessment-item' + (key === selectedPlanKey ? ' selected' : '');
      li.innerHTML = `
        <button type="button" data-key="${escapeHtml(key)}">
          <span class="name">${escapeHtml(p.namespace)}/${escapeHtml(p.name)}</span>
          <span class="phase">${escapeHtml(p.phase)}</span>
          <span class="score-mini">${p.waveCount ?? p.wave_count ?? 0} wave(s)</span>
        </button>
      `;
      li.querySelector('button').addEventListener('click', () => selectPlan(key));
      ul.appendChild(li);
    });
  }

  function renderWaves(waves) {
    const ul = $('plan-waves');
    if (!ul) return;
    ul.innerHTML = '';
    (waves || []).forEach((w) => {
      const li = document.createElement('li');
      li.className = 'wave-card';
      const prereq = (w.prerequisites || [])
        .map((x) => `<li>${escapeHtml(x)}</li>`)
        .join('');
      const tasks = (w.policyTasks || w.policy_tasks || [])
        .map(
          (t) =>
            `<li><strong>${escapeHtml(t.name)}</strong> (${escapeHtml(t.namespace)}): ${escapeHtml(t.action)}</li>`
        )
        .join('');
      li.innerHTML = `
        <h4>${escapeHtml(w.name)}</h4>
        <p class="ns-list">Namespaces: ${escapeHtml((w.namespaces || []).join(', ') || '—')}</p>
        ${prereq ? `<p><strong>Prerequisites</strong></p><ul>${prereq}</ul>` : ''}
        ${tasks ? `<p><strong>Policy tasks</strong></p><ul>${tasks}</ul>` : ''}
      `;
      ul.appendChild(li);
    });
  }

  function renderTranslations(translations) {
    const ul = $('plan-translations');
    if (!ul) return;
    ul.innerHTML = '';
    if (!translations || !translations.length) {
      ul.innerHTML = '<li class="hint">No PolicyTranslation resources in this namespace yet.</li>';
      return;
    }
    translations.forEach((t) => {
      const li = document.createElement('li');
      li.className = 'translation-card';
      const manifest = t.suggestedManifest || t.suggested_manifest;
      li.innerHTML = `
        <h4>${escapeHtml(t.sourceName || t.source_name)} → HTTPRoute</h4>
        <span class="phase-badge ${escapeHtml((t.phase || '').toLowerCase())}">${escapeHtml(t.phase)}</span>
        ${manifest ? `<pre>${escapeHtml(manifest)}</pre>` : '<p class="hint">No manifest yet</p>'}
      `;
      ul.appendChild(li);
    });
  }

  async function selectPlan(key) {
    selectedPlanKey = key;
    const p = plans.find((x) => planKey(x) === key);
    if (!p) return;
    renderPlanList();
    $('plan-detail-title').textContent = `${p.namespace}/${p.name}`;
    $('plan-detail-phase').textContent = p.phase;
    $('plan-detail-phase').className =
      'phase-badge ' + (p.phase || '').toLowerCase().replace(/[^a-z]/g, '');
    const ref = p.assessmentRef || p.assessment_ref;
    $('plan-assessment-ref').textContent = ref
      ? `Assessment: ${ref}`
      : 'No assessment reference';
    $('export-plan').disabled = false;
    $('start-rollout').disabled = false;
    renderWaves(p.waves);
    setStatus('Loading plan detail…');
    try {
      const res = await fetch(
        API() + `/api/v1/plans/${encodeURIComponent(p.namespace)}/${encodeURIComponent(p.name)}`
      );
      if (!res.ok) throw new Error(await res.text());
      const detail = await res.json();
      renderTranslations(detail.translations);
      setStatus(`Plan ${p.namespace}/${p.name} loaded`);
    } catch (e) {
      setStatus('Failed to load plan detail: ' + e.message, true);
      renderTranslations([]);
    }
    showPanel('plans');
  }

  async function loadPlans() {
    setStatus('Loading migration plans…');
    try {
      const res = await fetch(API() + '/api/v1/plans');
      if (!res.ok) throw new Error(await res.text());
      plans = await res.json();
      renderPlanList();
      setStatus(
        plans.length
          ? `Loaded ${plans.length} migration plan(s)`
          : 'No migration plans in cluster'
      );
      if (plans.length && !selectedPlanKey) {
        selectPlan(planKey(plans[0]));
      }
    } catch (e) {
      setStatus('Failed to load plans: ' + e.message, true);
    }
  }

  function rolloutKey(r) {
    return r.namespace + '/' + r.name;
  }

  function renderRolloutList() {
    const ul = $('rollout-list');
    if (!ul) return;
    ul.innerHTML = '';
    rollouts.forEach((r) => {
      const li = document.createElement('li');
      const key = rolloutKey(r);
      li.className = 'assessment-item' + (key === selectedRolloutKey ? ' selected' : '');
      const awaiting = r.awaitingApproval || r.awaiting_approval;
      li.innerHTML = `
        <button type="button" data-key="${escapeHtml(key)}">
          <span class="name">${escapeHtml(r.namespace)}/${escapeHtml(r.name)}</span>
          <span class="phase">${escapeHtml(r.phase)}</span>
          <span class="score-mini">stage ${r.currentStage ?? r.current_stage ?? 0}</span>
          ${awaiting ? '<span class="badge warn">approve</span>' : ''}
        </button>
      `;
      li.querySelector('button').addEventListener('click', () => selectRollout(key));
      ul.appendChild(li);
    });
  }

  function renderRolloutAudit(events) {
    const ul = $('rollout-audit-list');
    const hint = $('rollout-audit-hint');
    if (!ul) return;
    ul.innerHTML = '';
    if (!events || !events.length) {
      if (hint) {
        hint.textContent = events
          ? 'No audit events for this rollout yet.'
          : 'Audit log unavailable (configure DATABASE_URL on the API).';
      }
      return;
    }
    if (hint) hint.textContent = `${events.length} recent event(s)`;
    events.forEach((ev) => {
      const li = document.createElement('li');
      const ts = ev.timestamp ? new Date(ev.timestamp).toLocaleString() : '—';
      li.innerHTML = `
        <span class="audit-ts">${escapeHtml(ts)}</span>
        <span class="audit-action">${escapeHtml(ev.action)}</span>
        <span><span class="audit-outcome">${escapeHtml(ev.outcome)}</span> · ${escapeHtml(ev.actor || '')}${ev.details?.stageName ? ' · ' + escapeHtml(ev.details.stageName) : ''}</span>
      `;
      ul.appendChild(li);
    });
  }

  async function loadRolloutAudit(namespace, name) {
    try {
      const res = await fetch(
        API() +
          `/api/v1/rollouts/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}/audit?limit=50`
      );
      if (res.status === 503) {
        renderRolloutAudit(null);
        return;
      }
      if (!res.ok) throw new Error(await res.text());
      renderRolloutAudit(await res.json());
    } catch (e) {
      renderRolloutAudit([]);
      if ($('rollout-audit-hint')) {
        $('rollout-audit-hint').textContent = 'Could not load audit log: ' + e.message;
      }
    }
  }

  function renderRolloutStages(detail) {
    const tbody = $('rollout-stages')?.querySelector('tbody');
    if (!tbody) return;
    tbody.innerHTML = '';
    const current = detail.rollout?.currentStage ?? detail.rollout?.current_stage ?? detail.currentStage ?? detail.current_stage ?? 0;
    const awaiting = detail.rollout?.awaitingApproval ?? detail.rollout?.awaiting_approval ?? detail.awaitingApproval ?? detail.awaiting_approval;
    (detail.stages || []).forEach((s) => {
      const tr = document.createElement('tr');
      if (s.index === current) tr.classList.add('current');
      if (s.index === current && awaiting) tr.classList.add('awaiting');
      const approval = s.requiresApproval || s.requires_approval ? 'required' : 'auto';
      const result = s.resultPhase || s.result_phase || '—';
      tr.innerHTML = `
        <td>${s.index}</td>
        <td>${escapeHtml(s.name)}</td>
        <td>${escapeHtml(s.stageType || s.stage_type || '')}</td>
        <td>${approval}</td>
        <td>${escapeHtml(result)}${s.resultMessage || s.result_message ? ': ' + escapeHtml(s.resultMessage || s.result_message) : ''}</td>
      `;
      tbody.appendChild(tr);
    });
  }

  async function selectRollout(key) {
    selectedRolloutKey = key;
    const r = rollouts.find((x) => rolloutKey(x) === key);
    if (!r) return;
    renderRolloutList();
    $('rollout-detail-title').textContent = `${r.namespace}/${r.name}`;
    $('rollout-detail-phase').textContent = r.phase;
    $('rollout-detail-phase').className =
      'phase-badge ' + (r.phase || '').toLowerCase().replace(/[^a-z]/g, '');
    const current = r.currentStage ?? r.current_stage ?? 0;
    const total = r.stageCount ?? r.stage_count ?? '?';
    $('rollout-stage-progress').textContent = `Stage ${current} of ${total} · approved through ${r.approvedStage ?? r.approved_stage ?? 0}`;
    const awaiting = r.awaitingApproval || r.awaiting_approval;
    $('approve-rollout').disabled = !canApproveRollout(awaiting);
    updateApproveAuthHint();
    setStatus('Loading rollout detail…');
    try {
      const res = await fetch(
        API() + `/api/v1/rollouts/${encodeURIComponent(r.namespace)}/${encodeURIComponent(r.name)}`
      );
      if (!res.ok) throw new Error(await res.text());
      rolloutDetail = await res.json();
      renderRolloutStages(rolloutDetail);
      const awaitingDetail = rolloutDetail.rollout?.awaitingApproval ?? rolloutDetail.rollout?.awaiting_approval;
      $('approve-rollout').disabled = !canApproveRollout(awaitingDetail);
      updateApproveAuthHint();
      await loadRolloutAudit(r.namespace, r.name);
      setStatus(`Rollout ${r.namespace}/${r.name} loaded`);
    } catch (e) {
      setStatus('Failed to load rollout: ' + e.message, true);
      renderRolloutStages({ stages: [] });
    }
    showPanel('rollouts');
  }

  async function loadMeshInstances() {
    const el = $('mesh-instances-hint');
    if (!el || !API()) return;
    try {
      const res = await fetch(API() + '/api/v1/mesh-instances');
      if (!res.ok) throw new Error(await res.text());
      const items = await res.json();
      if (!items.length) {
        el.textContent = 'No Istio control planes discovered.';
        return;
      }
      const ambient = items.filter((m) => m.ambient);
      const lines = items.map((m) => {
        const mode = m.enrollment?.mode || 'unknown';
        const disc =
          m.enrollment?.discoveryLabelKey && m.enrollment?.discoveryLabelValue
            ? `${m.enrollment.discoveryLabelKey}=${m.enrollment.discoveryLabelValue}`
            : 'revision-only';
        const auto = m.autoSelect || m.auto_select ? ' (auto)' : '';
        return `${m.discoveryLabel || m.discovery_label}: rev=${m.revision}, ${disc}, mode=${mode}${auto}`;
      });
      el.textContent =
        (ambient.length === 1
          ? 'Single ambient mesh — rollouts can omit meshTarget. '
          : ambient.length > 1
            ? 'Multiple ambient meshes — set rollout.spec.meshTarget. '
            : '') + lines.join(' · ');
    } catch (e) {
      el.textContent = 'Mesh instances: ' + e.message;
    }
  }

  async function loadRollouts() {
    setStatus('Loading rollouts…');
    loadMeshInstances();
    try {
      const res = await fetch(API() + '/api/v1/rollouts');
      if (!res.ok) throw new Error(await res.text());
      rollouts = await res.json();
      renderRolloutList();
      setStatus(
        rollouts.length
          ? `Loaded ${rollouts.length} rollout(s)`
          : 'No rollouts in cluster (start one from a migration plan)'
      );
      if (rollouts.length && !selectedRolloutKey) {
        selectRollout(rolloutKey(rollouts[0]));
      }
    } catch (e) {
      setStatus('Failed to load rollouts: ' + e.message, true);
    }
  }

  async function approveCurrentRolloutStage() {
    const r = rollouts.find((x) => rolloutKey(x) === selectedRolloutKey);
    if (!r) return;
    const awaiting = r.awaitingApproval || r.awaiting_approval;
    if (!canApproveRollout(awaiting)) {
      setStatus(
        authConfig.requireAuthForApprove && !getToken()
          ? 'Sign in to approve rollout stages'
          : 'No stage awaiting approval',
        true
      );
      return;
    }
    $('approve-rollout').disabled = true;
    setStatus('Approving stage…');
    try {
      const res = await fetch(
        API() +
          `/api/v1/rollouts/${encodeURIComponent(r.namespace)}/${encodeURIComponent(r.name)}/approve`,
        {
          method: 'POST',
          headers: authHeaders({ 'Content-Type': 'application/json' }),
          body: getToken() ? '{}' : JSON.stringify({ actor: 'portal' }),
        }
      );
      if (!res.ok) throw new Error(await res.text());
      setStatus(`Approved stage for ${r.namespace}/${r.name}`);
      await loadRollouts();
      await selectRollout(rolloutKey(r));
    } catch (e) {
      setStatus('Approve failed: ' + e.message, true);
      $('approve-rollout').disabled = false;
    }
  }

  async function startRolloutFromPlan() {
    const p = plans.find((x) => planKey(x) === selectedPlanKey);
    if (!p) return;
    $('start-rollout').disabled = true;
    setStatus('Creating rollout…');
    try {
      const res = await fetch(
        API() +
          `/api/v1/plans/${encodeURIComponent(p.namespace)}/${encodeURIComponent(p.name)}/rollout`,
        { method: 'POST' }
      );
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      setStatus(`Created rollout ${data.namespace}/${data.name}`);
      selectedRolloutKey = data.namespace + '/' + data.name;
      showPanel('rollouts');
      await loadRollouts();
      await selectRollout(selectedRolloutKey);
    } catch (e) {
      setStatus('Start rollout failed: ' + e.message, true);
    } finally {
      $('start-rollout').disabled = false;
    }
  }

  async function downloadPlanExport() {
    const p = plans.find((x) => planKey(x) === selectedPlanKey);
    if (!p) return;
    setStatus('Generating export…');
    try {
      const url =
        API() +
        `/api/v1/plans/${encodeURIComponent(p.namespace)}/${encodeURIComponent(p.name)}/export`;
      const res = await fetch(url);
      if (!res.ok) throw new Error(await res.text());
      const yaml = await res.text();
      const blob = new Blob([yaml], { type: 'application/x-yaml' });
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = `${p.name}-export.yaml`;
      a.click();
      URL.revokeObjectURL(a.href);
      setStatus(`Exported ${p.name}-export.yaml`);
    } catch (e) {
      setStatus('Export failed: ' + e.message, true);
    }
  }

  function initNav() {
    document.querySelectorAll('nav a[href^="#"]').forEach((a) => {
      a.addEventListener('click', (e) => {
        e.preventDefault();
        const id = a.getAttribute('href').slice(1);
        showPanel(id);
        if (id === 'dashboard') loadDashboard();
        if (id === 'assessments') loadAssessments();
        if (id === 'plans') loadPlans();
        if (id === 'rollouts') loadRollouts();
      });
    });
  }

  function initSse() {
    if (!API()) return;
    const evtSource = new EventSource(API() + '/api/v1/events/assessment');
    evtSource.onmessage = (e) => appendEvent(e.data);
    evtSource.onerror = () => appendEvent('SSE connection error');
  }

  document.addEventListener('DOMContentLoaded', () => {
    consumeOidcTokenFromUrl();
    initNav();
    loadAuthConfig().then(() => {
      if (getToken()) setStatus('Signed in as ' + (parseJwtUsername(getToken()) || 'user'));
    });
    $('auth-login-btn')?.addEventListener('click', loginLocal);
    $('auth-logout-btn')?.addEventListener('click', logout);
    $('auth-oidc-login')?.addEventListener('click', startOidcLogin);
    $('run-assess')?.addEventListener('click', runAssessment);
    $('refresh-dashboard')?.addEventListener('click', loadDashboard);
    $('refresh-assessments')?.addEventListener('click', loadAssessments);
    $('app-detail-close')?.addEventListener('click', closeApplicationDetail);
    $('app-page-prev')?.addEventListener('click', () => {
      if (appListPage > 1) {
        appListPage -= 1;
        loadApplications();
      }
    });
    $('app-page-next')?.addEventListener('click', () => {
      appListPage += 1;
      loadApplications();
    });
    $('app-search')?.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        appListFilters.q = e.target.value.trim();
        appListPage = 1;
        loadApplications();
      }
    });
    $('app-risk-filter')?.addEventListener('change', (e) => {
      appListFilters.riskLevel = e.target.value;
      appListPage = 1;
      loadApplications();
    });
    $('app-mesh-filter')?.addEventListener('change', (e) => {
      appListFilters.meshRevision = e.target.value;
      appListPage = 1;
      loadApplications();
    });
    $('refresh-plans')?.addEventListener('click', loadPlans);
    $('export-plan')?.addEventListener('click', downloadPlanExport);
    $('start-rollout')?.addEventListener('click', startRolloutFromPlan);
    $('refresh-rollouts')?.addEventListener('click', loadRollouts);
    $('approve-rollout')?.addEventListener('click', approveCurrentRolloutStage);
    initSse();
    showPanel('dashboard');
    loadDashboard();
  });
})();
