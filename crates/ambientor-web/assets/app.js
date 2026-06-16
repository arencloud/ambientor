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
  let meshInstancesForPlan = [];
  let selectedMigrationNamespaces = new Set();
  let rollouts = [];
  let selectedRolloutKey = null;
  let rolloutDetail = null;
  let migrationPollTimer = null;
  let activeClusterRef = '';
  let fleetClusters = [];
  let clusterConnections = [];

  function isRemoteConnectionRef(ref) {
    return !!ref && ref.includes('/') && ref !== 'in-cluster';
  }

  function connectionAssessPath(ref) {
    const parts = ref.split('/');
    if (parts.length < 2) return null;
    const ns = encodeURIComponent(parts[0]);
    const name = encodeURIComponent(parts.slice(1).join('/'));
    return '/api/v1/connections/' + ns + '/' + name + '/assess';
  }

  function showPanel(id) {
    document.querySelectorAll('main .view-panel, main .panel').forEach((p) => p.classList.add('hidden'));
    const panel = document.getElementById(id);
    if (panel) panel.classList.remove('hidden');
    document.querySelectorAll('.nav-link, nav a').forEach((a) => {
      const href = a.getAttribute('href') || '';
      a.classList.toggle('active', href === '#' + id);
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

  function updateMigrationSelectionUi() {
    const count = selectedMigrationNamespaces.size;
    const countEl = $('plan-selection-count');
    if (countEl) countEl.textContent = `${count.toLocaleString()} selected`;
    const btn = $('create-migration-plan');
    if (btn) {
      btn.disabled = count === 0 || !$('plan-mesh-select')?.value;
    }
  }

  function meshOptionValue(m) {
    return JSON.stringify({
      revision: m.revision,
      discoveryLabel: m.discoveryLabel || m.discovery_label || '',
      controlPlaneNamespace: m.controlPlaneNamespace || m.control_plane_namespace,
    });
  }

  function renderPlanMeshSelect() {
    const select = $('plan-mesh-select');
    if (!select) return;
    const prev = select.value;
    const ambient = meshInstancesForPlan.filter((m) => m.ambient);
    const list = ambient.length ? ambient : meshInstancesForPlan;
    select.innerHTML = '';
    if (!list.length) {
      select.innerHTML = '<option value="">No control planes discovered</option>';
      return;
    }
    list.forEach((m) => {
      const opt = document.createElement('option');
      opt.value = meshOptionValue(m);
      const disc =
        m.enrollment?.discoveryLabelKey && m.enrollment?.discoveryLabelValue
          ? ` · ${m.enrollment.discoveryLabelKey}=${m.enrollment.discoveryLabelValue}`
          : '';
      const revTag = m.enrollment?.revisionTag || m.enrollment?.revision_tag;
      const revLabel = revTag || m.enrollment?.revision || m.revision;
      const tagHint = revTag ? ` · tag ${revTag}` : '';
      opt.textContent = `${revLabel} (${m.controlPlaneNamespace || m.control_plane_namespace})${tagHint}${disc}`;
      if (m.autoSelect || m.auto_select) opt.selected = true;
      select.appendChild(opt);
    });
    if (prev) select.value = prev;
    updatePlanLabelPreview();
    updateMigrationSelectionUi();
  }

  function updatePlanLabelPreview() {
    const el = $('plan-label-preview');
    const select = $('plan-mesh-select');
    if (!el || !select || !select.value) {
      if (el) el.textContent = 'Select a mesh to preview enrollment labels.';
      return;
    }
    let mesh;
    try {
      mesh = JSON.parse(select.value);
    } catch {
      el.textContent = 'Invalid mesh selection';
      return;
    }
    const instance = meshInstancesForPlan.find(
      (m) =>
        m.revision === mesh.revision &&
        (m.controlPlaneNamespace || m.control_plane_namespace) === mesh.controlPlaneNamespace
    );
    const revLabel =
      instance?.enrollment?.revisionTag ||
      instance?.enrollment?.revision_tag ||
      instance?.enrollment?.revision ||
      mesh.revision;
    const parts = [`istio.io/rev=${revLabel}`];
    if (instance?.enrollment?.discoveryLabelKey && instance?.enrollment?.discoveryLabelValue) {
      parts.push(
        `${instance.enrollment.discoveryLabelKey}=${instance.enrollment.discoveryLabelValue}`
      );
    } else if (mesh.discoveryLabel) {
      parts.push(`istio-discovery=${mesh.discoveryLabel}`);
    }
    parts.push('istio.io/dataplane-mode=ambient');
    const tagNote =
      instance?.enrollment?.revisionTag || instance?.enrollment?.revision_tag
        ? ` (istiod revision ${mesh.revision})`
        : '';
    el.textContent = `Rollout will enroll namespaces with: ${parts.join(', ')}${tagNote}`;
  }

  async function loadMeshInstancesForPlans() {
    if (!API()) return;
    try {
      const res = await fetch(API() + '/api/v1/mesh-instances');
      if (!res.ok) throw new Error(await res.text());
      meshInstancesForPlan = await res.json();
      renderPlanMeshSelect();
    } catch {
      meshInstancesForPlan = [];
      renderPlanMeshSelect();
    }
  }

  function toggleMigrationNamespace(ns, checked) {
    if (checked) selectedMigrationNamespaces.add(ns);
    else selectedMigrationNamespaces.delete(ns);
    updateMigrationSelectionUi();
  }

  function selectEligibleOnPage() {
    (applicationsPage.items || []).forEach((app) => {
      const blockers = app.blockerCount ?? app.blocker_count ?? 0;
      const candidate = app.migrationCandidate ?? app.migration_candidate ?? true;
      if (!blockers && candidate) selectedMigrationNamespaces.add(app.namespace);
    });
    renderApplicationsTable();
    updateMigrationSelectionUi();
  }

  function clearMigrationSelection() {
    selectedMigrationNamespaces.clear();
    renderApplicationsTable();
    updateMigrationSelectionUi();
  }

  async function createMigrationPlan() {
    const select = $('plan-mesh-select');
    if (!select?.value || selectedMigrationNamespaces.size === 0) return;
    let meshTarget;
    try {
      meshTarget = JSON.parse(select.value);
    } catch {
      setStatus('Invalid mesh selection', true);
      return;
    }
    const body = {
      displayName: $('plan-display-name')?.value?.trim() || undefined,
      meshTarget,
      selectedNamespaces: [...selectedMigrationNamespaces].sort(),
      clusterRef: applicationsPage.clusterRef || applicationsPage.cluster_ref,
    };
    const btn = $('create-migration-plan');
    if (btn) btn.disabled = true;
    setStatus('Creating migration plan…');
    try {
      const res = await fetch(API() + '/api/v1/plans', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...authHeaders() },
        body: JSON.stringify(body),
      });
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      setStatus(
        `Created plan ${data.namespace}/${data.name} (${data.selectedCount} namespaces, ${data.waveCount} waves)`
      );
      selectedPlanKey = `${data.namespace}/${data.name}`;
      showPanel('plans');
      await loadPlans();
      await selectPlan(selectedPlanKey);
    } catch (e) {
      setStatus('Create plan failed: ' + e.message, true);
    } finally {
      if (btn) btn.disabled = false;
      updateMigrationSelectionUi();
    }
  }

  function renderApplicationsTable() {
    const tbody = $('app-assess-tbody');
    if (!tbody) return;
    const items = applicationsPage.items || [];
    if (!items.length) {
        tbody.innerHTML =
        '<tr><td colspan="10" class="empty-cell">No migration candidates on this cluster. Run assessment to scan sidecar workloads.</td></tr>';
      return;
    }
    tbody.innerHTML = items
      .map((app) => {
        const ns = app.namespace;
        const selected = ns === selectedAppNamespace ? ' selected' : '';
        const readiness = app.readinessPct ?? app.readiness_pct ?? 0;
        const risk = app.riskLevel || app.risk_level || 'low';
        const blockers = app.blockerCount ?? app.blocker_count ?? 0;
        const warnings = app.warningCount ?? app.warning_count ?? 0;
        const dp = formatDataplane(app);
        const candidate = app.migrationCandidate ?? app.migration_candidate ?? true;
        const blocked = blockers > 0;
        const checked = selectedMigrationNamespaces.has(ns);
        const rowClass =
          'app-row' + selected + (blocked || !candidate ? ' row-blocked' : '');
        const checkCell =
          blocked || !candidate
            ? `<td class="col-check" title="${blocked ? 'Resolve blockers before migration' : 'Already on ambient dataplane'}">—</td>`
            : `<td class="col-check"><input type="checkbox" data-ns="${escapeHtml(ns)}" ${checked ? 'checked' : ''} aria-label="Select ${escapeHtml(appDisplayName(app))} for migration" /></td>`;
        const displayName = appDisplayName(app);
        return `<tr class="${rowClass}" data-ns="${escapeHtml(ns)}" tabindex="0">
          ${checkCell}
          <td><strong>${escapeHtml(displayName)}</strong></td>
          <td class="mono">${escapeHtml(ns)}</td>
          <td class="mono">${app.workloadCount ?? app.workload_count ?? '—'}</td>
          <td>${escapeHtml(formatControlPlane(app))}</td>
          <td><span class="badge-dataplane ${dataplaneBadgeClass(dp)}">${escapeHtml(dp)}</span></td>
          <td class="mono small">${escapeHtml(formatHostnames(app.hostnames))}</td>
          <td>${blockers ? `<span class="badge-status blocker">${blockers}</span>` : '—'}</td>
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
      row.querySelector('input[type="checkbox"]')?.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleMigrationNamespace(ns, e.target.checked);
      });
      row.addEventListener('click', (e) => {
        if (e.target.closest('input[type="checkbox"]')) return;
        openApplicationDetail(ns);
      });
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          openApplicationDetail(ns);
        }
      });
    });
    updateMigrationSelectionUi();
  }

  function updatePaginationUi() {
    const total = applicationsPage.total || 0;
    const page = applicationsPage.page || appListPage;
    const pageSize = applicationsPage.pageSize || applicationsPage.page_size || 50;
    const pages = Math.max(1, Math.ceil(total / pageSize));
    const info = $('app-page-info');
    if (info) {
      info.textContent = total
        ? `Page ${page} of ${pages} · ${total.toLocaleString()} candidate(s)`
        : 'No candidates';
    }
    const prev = $('app-page-prev');
    const next = $('app-page-next');
    if (prev) prev.disabled = page <= 1;
    if (next) next.disabled = page >= pages;
  }

  function clusterQuerySuffix() {
    return activeClusterRef ? '&clusterRef=' + encodeURIComponent(activeClusterRef) : '';
  }

  function clusterQueryPrefix() {
    return activeClusterRef ? '?clusterRef=' + encodeURIComponent(activeClusterRef) : '';
  }

  async function loadFleetClusters() {
    const select = $('cluster-select');
    if (!select || !API()) return;
    try {
      const [fleetRes, connRes] = await Promise.all([
        fetch(API() + '/api/v1/dashboard/fleet'),
        fetch(API() + '/api/v1/connections'),
      ]);
      if (fleetRes.ok) {
        const fleet = await fleetRes.json();
        fleetClusters = fleet.clusters || fleet.Clusters || [];
      }
      if (connRes.ok) {
        clusterConnections = await connRes.json();
      }
      const prev = select.value;
      select.innerHTML = '<option value="">Hub (local)</option>';
      fleetClusters.forEach((c) => {
        const ref = c.clusterRef || c.cluster_ref;
        if (!ref) return;
        const opt = document.createElement('option');
        opt.value = ref;
        const name = c.cluster?.name || ref;
        opt.textContent = name + (ref !== name ? ` (${ref})` : '');
        select.appendChild(opt);
      });
      const remote = clusterConnections.filter((c) => !c.hub);
      if (remote.length) {
        const group = document.createElement('optgroup');
        group.label = 'Remote connections';
        remote.forEach((c) => {
          const ref = c.namespace + '/' + c.name;
          const opt = document.createElement('option');
          opt.value = ref;
          const label = c.displayName || c.display_name || c.name;
          const phase = c.phase || 'Unknown';
          opt.textContent = label + ' · ' + phase;
          group.appendChild(opt);
        });
        select.appendChild(group);
      }
      if (prev) select.value = prev;
      else if (activeClusterRef) select.value = activeClusterRef;
      renderConnectionsList();
    } catch {
      /* fleet / connections API optional */
    }
  }

  function renderConnectionsList() {
    const list = $('dash-connections-list');
    if (!list) return;
    const remote = clusterConnections.filter((c) => !c.hub);
    if (!remote.length) {
      list.innerHTML =
        '<p class="hint">No remote <code>ClusterConnection</code> resources. Register spoke clusters on the hub to assess them from this portal.</p>';
      return;
    }
    list.innerHTML = remote
      .map((c) => {
        const ref = c.namespace + '/' + c.name;
        const label = escapeHtml(c.displayName || c.display_name || c.name);
        const phase = escapeHtml(c.phase || 'Unknown');
        const msg = c.readyMessage || c.ready_message;
        const hint = msg ? `<p class="hint small">${escapeHtml(msg)}</p>` : '';
        const active = ref === activeClusterRef ? ' connection-row-active' : '';
        return `<article class="connection-row${active}">
          <div>
            <strong>${label}</strong>
            <span class="mono small">${escapeHtml(ref)}</span>
            ${hint}
          </div>
          <span class="badge-status ${statusCssClass(phase)}">${phase}</span>
        </article>`;
      })
      .join('');
  }

  function onClusterChange(ref) {
    activeClusterRef = ref || '';
    const bar = $('cluster-bar');
    if (bar) bar.classList.toggle('cluster-active', !!activeClusterRef);
    renderConnectionsList();
    loadDashboard();
    if (!$('assessments')?.classList.contains('hidden')) loadApplications();
    if (!$('rollouts')?.classList.contains('hidden')) loadRollouts();
  }

  function applicationsQueryString() {
    const params = new URLSearchParams();
    params.set('page', String(appListPage));
    params.set('pageSize', '50');
    if (activeClusterRef) params.set('clusterRef', activeClusterRef);
    if (appListFilters.q) params.set('q', appListFilters.q);
    if (appListFilters.riskLevel) params.set('riskLevel', appListFilters.riskLevel);
    if (appListFilters.meshRevision) params.set('meshRevision', appListFilters.meshRevision);
    params.set('migrationCandidatesOnly', 'true');
    return params.toString();
  }

  function appDisplayName(app) {
    return app.applicationName || app.application_name || app.namespace;
  }

  async function loadApplications() {
    setStatus('Loading migration candidates…');
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
          ? `${applicationsPage.total.toLocaleString()} sidecar migration candidate(s) · assessed ${new Date(when).toLocaleString()}`
          : applicationsPage.total
            ? `${applicationsPage.total.toLocaleString()} candidate(s) — run assessment again to refresh`
            : 'No migration candidates yet. Run assessment on this cluster to discover sidecar workloads.';
      }
      renderMeshFilterOptions();
      renderApplicationsTable();
      updatePaginationUi();
      setStatus(`Loaded ${applicationsPage.total.toLocaleString()} migration candidate(s)`);
    } catch (e) {
      setStatus('Failed to load migration candidates: ' + e.message, true);
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
        API() +
          '/api/v1/applications/' +
          encodeURIComponent(namespace) +
          (activeClusterRef ? '?clusterRef=' + encodeURIComponent(activeClusterRef) : '')
      );
      if (!res.ok) throw new Error(await res.text());
      const detail = await res.json();
      const app = detail.list || detail;
      $('app-detail-title').textContent = appDisplayName(app);
      $('app-detail-meta').innerHTML = `
        <dl class="meta-dl">
          <dt>Application</dt><dd><strong>${escapeHtml(appDisplayName(app))}</strong></dd>
          <dt>Namespace</dt><dd class="mono">${escapeHtml(app.namespace)}</dd>
          <dt>Pods</dt><dd>${app.workloadCount ?? app.workload_count ?? 0}</dd>
          <dt>Control plane</dt><dd>${escapeHtml(formatControlPlane(app))}</dd>
          <dt>Revision NS</dt><dd>${escapeHtml(app.controlPlaneNamespace || app.control_plane_namespace || '—')}</dd>
          <dt>Hostnames</dt><dd class="mono">${escapeHtml((app.hostnames || []).join(', ') || '—')}</dd>
          <dt>Dataplane</dt><dd><span class="badge-dataplane ${dataplaneBadgeClass(formatDataplane(app))}">${escapeHtml(formatDataplane(app))}</span></dd>
          <dt>Istio labels</dt><dd class="mono small">${escapeHtml(formatLabels(app.namespaceLabels || app.namespace_labels))}</dd>
          <dt>Ingress gateway</dt><dd>${escapeHtml(formatIngress(app))}</dd>
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

  function renderMigrationSavings(savings) {
    const panel = $('dash-savings-panel');
    if (!panel) return;
    const s = savings || {};
    const workloads =
      s.migratedWorkloads ?? s.migrated_workloads ?? 0;
    if (!workloads) {
      panel.classList.add('hidden');
      panel.innerHTML = '';
      return;
    }
    const proxies =
      s.estimatedSidecarProxiesRemoved ?? s.estimated_sidecar_proxies_removed ?? workloads;
    const mem = s.estimatedMemoryMibSaved ?? s.estimated_memory_mib_saved ?? 0;
    const cpu = s.estimatedCpuMillicoresSaved ?? s.estimated_cpu_millicores_saved ?? 0;
    panel.classList.remove('hidden');
    panel.innerHTML = `
      <div class="dash-savings-inner">
        <div>
          <p class="dash-eyebrow">After migration</p>
          <h3>Estimated resource savings</h3>
          <p class="hint">Based on ${workloads} migrated workload(s) with ambient dataplane (sidecar proxies removed).</p>
        </div>
        <div class="savings-metrics" role="list">
          <div class="savings-metric" role="listitem">
            <span class="savings-value">${proxies}</span>
            <span class="savings-label">Sidecars removed</span>
          </div>
          <div class="savings-metric" role="listitem">
            <span class="savings-value">~${mem}</span>
            <span class="savings-label">MiB memory</span>
          </div>
          <div class="savings-metric" role="listitem">
            <span class="savings-value">~${cpu}</span>
            <span class="savings-label">mCPU</span>
          </div>
        </div>
      </div>`;
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
        const appName = app.applicationName || app.application_name || app.namespace;
        return `<tr>
          <td><strong>${escapeHtml(appName)}</strong><br><span class="mono small">${escapeHtml(app.namespace)}</span></td>
          <td><span class="badge-status ${statusCssClass(st)}">${escapeHtml(statusLabel(st))}</span></td>
          <td><span class="badge-dataplane ${dataplaneBadgeClass(dp)}">${escapeHtml(dp)}</span></td>
          <td>${escapeHtml(assess)}</td>
        </tr>`;
      })
      .join('');

    const kind = mesh.ambient ? 'ambient' : 'sidecar';
    const enroll = mesh.enrollment || {};
    const revTag = enroll.revisionTag || enroll.revision_tag;
    const revLine = revTag
      ? `<code>istio.io/rev=${escapeHtml(revTag)}</code> · istiod <code>${escapeHtml(enroll.istioRevision || enroll.istio_revision || mesh.revision)}</code>`
      : `revision <code>${escapeHtml(mesh.revision)}</code>`;
    card.innerHTML = `
      <div class="istiod-card-head">
        <div>
          <h4>${escapeHtml(mesh.discoveryLabel || mesh.discovery_label)}</h4>
          <p class="istiod-sub">${revLine} · ns <code>${escapeHtml(mesh.controlPlaneNamespace || mesh.control_plane_namespace)}</code> · ${kind}</p>
        </div>
        <div class="istiod-counts">${pills.join('') || '<span class="pill">No applications</span>'}</div>
      </div>
      <table class="app-table">
        <thead><tr><th>Application</th><th>Status</th><th>Dataplane</th><th>Assessment</th></tr></thead>
        <tbody>${rows || '<tr><td colspan="4">No enrolled namespaces on this control plane</td></tr>'}</tbody>
      </table>
    `;
    return card;
  }

  async function loadDashboard(quiet) {
    if (!quiet) setStatus('Loading dashboard…');
    const container = $('dash-mesh-instances');
    try {
      const res = await fetch(
        API() + '/api/v1/dashboard?fresh=true' + clusterQuerySuffix()
      );
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
      renderMigrationSavings(data.migrationSavings || data.migration_savings);
      if (container) {
        container.innerHTML = '';
        (data.meshInstances || data.mesh_instances || []).forEach((m) => {
          container.appendChild(renderIstiodCard(m));
        });
        if (!(data.meshInstances || data.mesh_instances || []).length) {
          container.innerHTML = '<p class="hint">No Istio control planes discovered.</p>';
        }
      }
      if (!quiet) setStatus('Dashboard loaded');
    } catch (e) {
      if (container) container.innerHTML = '';
      if (!quiet) setStatus('Dashboard failed: ' + e.message, true);
    }
  }

  function activeRollouts() {
    return rollouts.filter((r) => rolloutIsActive(r.phase));
  }

  function renderDashboardMigrationBanner(active) {
    const banner = $('dash-migration-banner');
    if (!banner) return;
    if (!active.length) {
      banner.classList.add('hidden');
      return;
    }
    banner.classList.remove('hidden');
    const r = active[0];
    const rolloutCtx = rolloutDetailFor(r);
    const pct = rolloutProgressPct(r, rolloutCtx);
    const stageLabel = rolloutStageLabel(r, rolloutCtx);
    const title = $('dash-migration-title');
    const detailEl = $('dash-migration-detail');
    const fill = $('dash-migration-progress');
    if (title) {
      title.textContent =
        active.length > 1
          ? `${active.length} migrations in progress`
          : `Migration in progress · ${r.namespace}/${r.name}`;
    }
    if (detailEl) {
      const awaiting = r.awaitingApproval || r.awaiting_approval;
      detailEl.textContent = `${stageLabel} · ${r.phase}${awaiting ? ' · awaiting approval' : ''}`;
    }
    if (fill) fill.style.width = `${pct}%`;
  }

  function migrationLiveEnabled() {
    return (
      $('dash-auto-refresh')?.checked !== false ||
      $('rollout-auto-refresh')?.checked !== false ||
      $('plan-auto-refresh')?.checked !== false
    );
  }

  function migrationStillActive() {
    const active = activeRollouts();
    if (active.length) return true;
    if (rolloutIsActive(rolloutDetail?.rollout?.phase)) return true;
    return planNeedsLiveRefresh();
  }

  function stopMigrationPolling() {
    if (migrationPollTimer) {
      clearInterval(migrationPollTimer);
      migrationPollTimer = null;
    }
  }

  async function pollMigrationActivity(quiet) {
    try {
      const res = await fetch(API() + '/api/v1/rollouts' + clusterQueryPrefix());
      if (!res.ok) return;
      rollouts = await res.json();
      const active = activeRollouts();
      renderDashboardMigrationBanner(active);

      const dashLive = $('dash-auto-refresh')?.checked !== false;
      const rolloutLive = $('rollout-auto-refresh')?.checked !== false;
      const planLive = $('plan-auto-refresh')?.checked !== false;

      if (dashLive) await loadDashboard(quiet);
      if (rolloutLive) {
        renderRolloutList();
        if (selectedRolloutKey) {
          const r = rollouts.find((x) => rolloutKey(x) === selectedRolloutKey);
          if (r) await refreshRolloutDetail(r, true);
        }
      }
      if (planLive) await refreshPlansQuiet();

      if (!migrationStillActive()) stopMigrationPolling();
    } catch (_) {}
  }

  function startMigrationPolling() {
    stopMigrationPolling();
    if (!migrationLiveEnabled()) return;
    if (!migrationStillActive()) return;
    renderDashboardMigrationBanner(activeRollouts());
    migrationPollTimer = setInterval(() => pollMigrationActivity(true), 5000);
  }

  function startDashboardPolling() {
    startMigrationPolling();
  }

  async function ensureMigrationPolling() {
    if (!rollouts.length) {
      try {
        const res = await fetch(API() + '/api/v1/rollouts' + clusterQueryPrefix());
        if (res.ok) rollouts = await res.json();
      } catch (_) {}
    }
    if (!plans.length) {
      try {
        const res = await fetch(API() + '/api/v1/plans');
        if (res.ok) plans = await res.json();
      } catch (_) {}
    }
    renderDashboardMigrationBanner(activeRollouts());
    startMigrationPolling();
  }

  async function runAssessment() {
    const remote = isRemoteConnectionRef(activeClusterRef);
    const assessPath = remote
      ? connectionAssessPath(activeClusterRef)
      : '/api/v1/assess';
    if (!assessPath) {
      setStatus('Invalid remote cluster selection', true);
      return;
    }
    setStatus(remote ? 'Running assessment on remote cluster…' : 'Running assessment…');
    const btn = $('run-assess');
    if (btn) btn.disabled = true;
    try {
      const res = await fetch(API() + assessPath, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      const count = data.applicationCount ?? data.application_count ?? 0;
      const trigger = data.trigger || (remote ? 'direct' : 'crd');
      const assessName = data.assessmentName || data.assessment_name;
      const assessNs = data.assessmentNamespace || data.assessment_namespace;
      const target = remote ? activeClusterRef : 'hub';
      const crdHint =
        trigger === 'crd' && assessName
          ? ` via ${assessNs || 'ambientor-system'}/${assessName}`
          : trigger === 'direct'
            ? remote
              ? ' (remote scan)'
              : ' (direct API scan)'
            : '';
      setStatus(
        `Assessment complete on ${target}${crdHint} — ${count.toLocaleString()} application(s) in database`
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

  function rolloutNameForPlan(p) {
    return `${p.name}-rollout`;
  }

  function rolloutForPlan(p) {
    const rolloutName = rolloutNameForPlan(p);
    return rollouts.find((r) => r.namespace === p.namespace && r.name === rolloutName);
  }

  function planIsBuilding(phase) {
    const p = (phase || '').toLowerCase();
    return p === 'pending' || p === 'processing' || p === 'running';
  }

  function planNeedsLiveRefresh() {
    return plans.some((p) => {
      if (planIsBuilding(p.phase)) return true;
      const r = rolloutForPlan(p);
      return r && rolloutIsActive(r.phase);
    });
  }

  function renderPlanList() {
    const ul = $('plan-list');
    if (!ul) return;
    ul.innerHTML = '';
    plans.forEach((p) => {
      const li = document.createElement('li');
      const key = planKey(p);
      li.className = key === selectedPlanKey ? 'selected' : '';
      const sel = (p.selectedNamespaces || p.selected_namespaces || []).length;
      const subtitle = sel
        ? `${sel} app(s)`
        : p.assessmentRef || p.assessment_ref
          ? 'assessment'
          : '';
      const approved = p.approved ? '<span class="badge-status success">Approved</span>' : '';
      li.innerHTML = `
        <button type="button" data-key="${escapeHtml(key)}">
          <span class="name">${escapeHtml(p.displayName || p.display_name || p.name)}</span>
          <span class="phase">${escapeHtml(p.phase)}</span>
          ${approved}
          <span class="score-mini">${p.waveCount ?? p.wave_count ?? 0} wave(s)${subtitle ? ' · ' + escapeHtml(subtitle) : ''}</span>
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

  function renderPlanSync(sync, plan) {
    const panel = $('plan-sync-panel');
    if (!panel) return;
    if (!sync) {
      panel.classList.add('hidden');
      return;
    }
    panel.classList.remove('hidden');
    const rollout = sync.rolloutPhase || sync.rollout_phase;
    const action = sync.nextAction || sync.next_action || 'execute';
    const lines = [];
    if (sync.rolloutName || sync.rollout_name) {
      lines.push(`Rollout: ${sync.rolloutName || sync.rollout_name} · ${rollout || '—'}`);
    } else {
      lines.push('Rollout: not created yet');
    }
    const actionText = {
      execute: 'Ready — one approval starts the full pipeline',
      approve_rollout: 'Rollout waiting for approval (use button below or GitOps patch)',
      running: 'Migration running automatically',
      completed: 'Migration completed',
      failed: 'Rollout failed — check stages or rollback',
      wait_plan: 'Plan not Ready yet',
    };
    lines.push(actionText[action] || action);
    $('plan-sync-summary').textContent = lines.join(' · ');
    const fill = $('plan-sync-progress');
    if (fill) {
      const linked = plan ? rolloutForPlan(plan) : null;
      if (linked) {
        fill.style.width = `${rolloutProgressPct(linked, rolloutDetailFor(linked))}%`;
      } else if (action === 'completed') {
        fill.style.width = '100%';
      } else {
        fill.style.width = '0%';
      }
    }
    const ch = sync.channels || {};
    $('plan-sync-cli').textContent = ch.cli || '';
    $('plan-sync-gitops-plan').textContent = ch.gitopsPlanPatch || ch.gitops_plan_patch || '';
    $('plan-sync-gitops-rollout').textContent =
      ch.gitopsRolloutPatch || ch.gitops_rollout_patch || '';
    const execBtn = $('execute-migration');
    if (execBtn) {
      const canExecute =
        action === 'execute' || action === 'approve_rollout';
      execBtn.disabled = !canExecute;
      execBtn.textContent =
        action === 'completed'
          ? 'Completed'
          : action === 'running'
            ? 'Running…'
            : 'Approve & run migration';
    }
  }

  function renderPlanSummary(p) {
    $('plan-detail-title').textContent = `${p.namespace}/${p.name}`;
    $('plan-detail-phase').textContent = p.phase;
    $('plan-detail-phase').className =
      'phase-badge ' + (p.phase || '').toLowerCase().replace(/[^a-z]/g, '');
    const ref = p.assessmentRef || p.assessment_ref;
    const selected = p.selectedNamespaces || p.selected_namespaces || [];
    const mesh = p.meshTarget || p.mesh_target;
    const clusterRef = p.clusterRef || p.cluster_ref;
    const displayName = p.displayName || p.display_name;
    const meta = $('plan-meta-grid');
    if (meta) {
      const meshLine = mesh
        ? `rev=${mesh.revision || '—'}${mesh.discoveryLabel || mesh.discovery_label ? ', discovery=' + (mesh.discoveryLabel || mesh.discovery_label) : ''}, CP=${mesh.controlPlaneNamespace || mesh.control_plane_namespace || '—'}`
        : '—';
      meta.innerHTML = `
        <div><span class="meta-label">Display name</span><span>${escapeHtml(displayName || '—')}</span></div>
        <div><span class="meta-label">Cluster</span><span>${escapeHtml(clusterRef || '—')}</span></div>
        <div><span class="meta-label">Mesh target</span><span class="mono">${escapeHtml(meshLine)}</span></div>
        <div><span class="meta-label">Selected apps</span><span>${selected.length ? selected.length.toLocaleString() : '—'}</span></div>
      `;
    }
    $('plan-assessment-ref').textContent = ref
      ? `Linked assessment (legacy): ${ref}`
      : selected.length
        ? `${selected.length.toLocaleString()} namespace(s) in spec.selectedNamespaces`
        : 'Assessment-wide plan (legacy)';
    $('export-plan').disabled = false;
    $('start-rollout').disabled = p.phase !== 'Ready';
  }

  async function refreshPlanDetail(p, quiet) {
    renderPlanSummary(p);
    try {
      const res = await fetch(
        API() + `/api/v1/plans/${encodeURIComponent(p.namespace)}/${encodeURIComponent(p.name)}`
      );
      if (!res.ok) throw new Error(await res.text());
      const detail = await res.json();
      if (!quiet || planIsBuilding(p.phase)) {
        renderTranslations(detail.translations);
      }
      renderPlanSync(detail.sync, p);
      if (!quiet) setStatus(`Plan ${p.namespace}/${p.name} loaded`);
      if (!quiet) startMigrationPolling();
    } catch (e) {
      if (!quiet) setStatus('Failed to load plan detail: ' + e.message, true);
      if (!quiet) {
        renderTranslations([]);
        renderPlanSync(null);
      }
    }
  }

  async function selectPlan(key) {
    selectedPlanKey = key;
    const p = plans.find((x) => planKey(x) === key);
    if (!p) return;
    renderPlanList();
    renderWaves(p.waves);
    await refreshPlanDetail(p, false);
    showPanel('plans');
  }

  async function refreshPlansQuiet() {
    try {
      const res = await fetch(API() + '/api/v1/plans');
      if (!res.ok) return;
      plans = await res.json();
      renderPlanList();
      if (selectedPlanKey) {
        const p = plans.find((x) => planKey(x) === selectedPlanKey);
        if (p) await refreshPlanDetail(p, true);
      }
    } catch (_) {}
  }

  async function loadPlans(quiet) {
    if (!quiet) setStatus('Loading migration plans…');
    try {
      const res = await fetch(API() + '/api/v1/plans');
      if (!res.ok) throw new Error(await res.text());
      plans = await res.json();
      renderPlanList();
      if (!quiet) {
        setStatus(
          plans.length
            ? `Loaded ${plans.length} migration plan(s)`
            : 'No migration plans in cluster'
        );
      }
      if (plans.length && !selectedPlanKey) {
        await selectPlan(planKey(plans[0]));
      } else {
        startMigrationPolling();
      }
    } catch (e) {
      if (!quiet) setStatus('Failed to load plans: ' + e.message, true);
    }
  }

  function rolloutKey(r) {
    return r.namespace + '/' + r.name;
  }

  function rolloutIsActive(phase) {
    const p = (phase || '').toLowerCase();
    return (
      p === 'running' ||
      p === 'awaitingapproval' ||
      p === 'pending' ||
      p === 'processing'
    );
  }

  function startRolloutPolling() {
    startMigrationPolling();
  }

  function rolloutPhaseMeta(phase) {
    const key = (phase || 'unknown').toLowerCase().replace(/[^a-z]/g, '');
    const map = {
      completed: { label: 'Completed', icon: '✓', class: 'completed' },
      running: { label: 'Running', icon: '↻', class: 'running' },
      pending: { label: 'Pending', icon: '○', class: 'pending' },
      awaitingapproval: { label: 'Awaiting approval', icon: '!', class: 'awaitingapproval' },
      failed: { label: 'Failed', icon: '✕', class: 'failed' },
      rolledback: { label: 'Rolled back', icon: '↩', class: 'rolledback' },
      processing: { label: 'Processing', icon: '↻', class: 'processing' },
    };
    return map[key] || { label: phase || 'Unknown', icon: '·', class: key || 'unknown' };
  }

  function stageTypeIcon(type) {
    const t = (type || '').toLowerCase();
    if (t.includes('enroll')) return '⎔';
    if (t.includes('restart')) return '↻';
    if (t.includes('verify')) return '✓';
    if (t.includes('label')) return '⌁';
    if (t.includes('waypoint')) return '◎';
    if (t.includes('remove')) return '⊖';
    return '●';
  }

  function stageResultDone(result) {
    const p = (result || '').toLowerCase();
    return p === 'succeeded' || p === 'completed';
  }

  function rolloutPhase(r, detail) {
    return (r?.phase || detail?.rollout?.phase || '').toLowerCase();
  }

  function rolloutStageTotal(r, detail) {
    const stages = detail?.stages;
    if (stages?.length) return stages.length;
    const n = r?.stageCount ?? r?.stage_count ?? 0;
    return n || 0;
  }

  function rolloutDetailFor(r) {
    if (!r || !rolloutDetail) return null;
    if (rolloutKey(r) !== selectedRolloutKey) return null;
    const ro = rolloutDetail.rollout;
    if (ro && rolloutKey(ro) === rolloutKey(r)) return rolloutDetail;
    return rolloutDetail;
  }

  function rolloutCompletedStages(r, detail) {
    const total = rolloutStageTotal(r, detail);
    const phase = rolloutPhase(r, detail);
    if (phase === 'completed') return total;
    const stages = detail?.stages || [];
    if (stages.length) {
      return stages.filter((s) => stageResultDone(s.resultPhase || s.result_phase)).length;
    }
    const current = r?.currentStage ?? r?.current_stage ?? 0;
    if (total > 0 && current >= total) return total;
    return Math.max(0, current);
  }

  function rolloutProgressPct(r, detail) {
    const total = rolloutStageTotal(r, detail);
    if (!total) return 0;
    if (rolloutPhase(r, detail) === 'completed') return 100;
    const done = rolloutCompletedStages(r, detail);
    return Math.min(100, Math.round((done / total) * 100));
  }

  function rolloutStageLabel(r, detail) {
    const total = rolloutStageTotal(r, detail);
    const done = rolloutCompletedStages(r, detail);
    const phase = rolloutPhase(r, detail);
    if (phase === 'completed') return `${total} stages complete`;
    if (!total) return '—';
    const active = rolloutIsActive(r?.phase);
    if (active && done < total) return `Stage ${done + 1} of ${total}`;
    return `${done} of ${total} stages done`;
  }

  function setRolloutDetailVisible(visible) {
    $('rollout-empty-state')?.classList.toggle('hidden', visible);
    $('rollout-detail-panel')?.classList.toggle('hidden', !visible);
  }

  function renderRolloutStats() {
    const bar = $('rollout-stats');
    if (!bar) return;
    const total = rollouts.length;
    const active = rollouts.filter((r) => rolloutIsActive(r.phase)).length;
    const completed = rollouts.filter(
      (r) => (r.phase || '').toLowerCase() === 'completed'
    ).length;
    const failed = rollouts.filter((r) => {
      const p = (r.phase || '').toLowerCase();
      return p === 'failed' || p === 'rolledback';
    }).length;
    const badge = $('rollout-count-badge');
    if (badge) badge.textContent = String(total);
    bar.innerHTML = `
      <div class="rollout-stat total"><span class="rollout-stat-value">${total}</span><span class="rollout-stat-label">Total</span></div>
      <div class="rollout-stat active"><span class="rollout-stat-value">${active}</span><span class="rollout-stat-label">In progress</span></div>
      <div class="rollout-stat completed"><span class="rollout-stat-value">${completed}</span><span class="rollout-stat-label">Completed</span></div>
      <div class="rollout-stat failed"><span class="rollout-stat-value">${failed}</span><span class="rollout-stat-label">Failed</span></div>
    `;
  }

  function renderRolloutList() {
    const ul = $('rollout-list');
    if (!ul) return;
    renderRolloutStats();
    const filter = ($('rollout-filter')?.value || '').toLowerCase();
    ul.innerHTML = '';
    const list = rollouts.filter((r) => {
      if (!filter) return true;
      const hay = `${r.namespace}/${r.name} ${r.phase} ${r.planRef || r.plan_ref || ''}`.toLowerCase();
      return hay.includes(filter);
    });
    if (!list.length) {
      ul.innerHTML = '<li class="hint rollout-list-empty">No rollouts match your search</li>';
      if (!selectedRolloutKey) setRolloutDetailVisible(false);
      return;
    }
    list.forEach((r) => {
      const li = document.createElement('li');
      const key = rolloutKey(r);
      li.className =
        'rollout-list-item' + (key === selectedRolloutKey ? ' selected' : '');
      const awaiting = r.awaitingApproval || r.awaiting_approval;
      const meta = rolloutPhaseMeta(r.phase);
      const detail = rolloutDetailFor(r);
      const pct = rolloutProgressPct(r, detail);
      const stageLabel = rolloutStageLabel(r, detail);
      const planRef = r.planRef || r.plan_ref;
      li.innerHTML = `
        <button type="button" data-key="${escapeHtml(key)}">
          <div class="rollout-card-top">
            <div>
              <span class="rollout-list-name">${escapeHtml(r.name)}</span>
              <span class="rollout-list-plan">${escapeHtml(r.namespace)}${planRef ? ' · ' + escapeHtml(planRef) : ''}</span>
            </div>
            <span class="phase-badge small ${meta.class}">${escapeHtml(meta.label)}</span>
          </div>
          <div class="rollout-card-progress">
            <div class="progress-track"><span style="display:block;height:100%;width:${pct}%;background:linear-gradient(90deg,var(--accent),var(--success));border-radius:3px;transition:width 0.4s ease"></span></div>
            <span>${pct}%</span>
          </div>
          <div class="rollout-list-meta">
            <span>${escapeHtml(stageLabel)}</span>
            ${awaiting ? '<span class="badge-status warning">Needs approval</span>' : ''}
          </div>
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
          ? '· no events yet'
          : '· audit unavailable';
      }
      return;
    }
    if (hint) hint.textContent = `· ${events.length} event(s)`;
    events.forEach((ev) => {
      const li = document.createElement('li');
      const ts = ev.timestamp ? new Date(ev.timestamp).toLocaleString() : '—';
      const outcome = (ev.outcome || '').toLowerCase();
      const dotClass = outcome === 'succeeded' ? 'success' : outcome === 'failed' ? 'failed' : '';
      li.innerHTML = `
        <span class="audit-dot ${dotClass}" aria-hidden="true"></span>
        <div>
          <div class="audit-event-head">
            <span class="audit-event-action">${escapeHtml(ev.action)}</span>
            <span class="audit-event-outcome ${escapeHtml(outcome)}">${escapeHtml(ev.outcome || '')}</span>
            <span class="audit-event-meta">${escapeHtml(ts)}</span>
          </div>
          <div class="audit-event-meta">${escapeHtml(ev.actor || 'system')}${ev.details?.stageName ? ' · ' + escapeHtml(ev.details.stageName) : ''}</div>
        </div>
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

  function renderRolloutMeshTarget(detail) {
    const el = $('rollout-mesh-target');
    if (!el) return;
    const mesh =
      detail.resolvedMeshTarget ||
      detail.resolved_mesh_target ||
      detail.rollout?.resolvedMeshTarget;
    if (!mesh) {
      el.classList.add('hidden');
      el.innerHTML = '';
      return;
    }
    el.classList.remove('hidden');
    const enroll = mesh.enrollment || {};
    const rev =
      enroll.revisionTag ||
      enroll.revision_tag ||
      enroll.revision ||
      mesh.revision;
    const chips = [`istio.io/rev=${rev}`, 'istio.io/dataplane-mode=ambient'];
    if (enroll.discoveryLabelKey && enroll.discoveryLabelValue) {
      chips.push(`${enroll.discoveryLabelKey}=${enroll.discoveryLabelValue}`);
    }
    el.innerHTML = `
      <strong>Target mesh</strong>
      <div>${escapeHtml(mesh.discoveryLabel || mesh.discovery_label || mesh.revision || 'Istio')}</div>
      <div class="rollout-mesh-chips">${chips.map((c) => `<span class="rollout-label-chip">${escapeHtml(c)}</span>`).join('')}</div>
    `;
  }

  function renderRolloutConditions(detail) {
    const el = $('rollout-conditions');
    if (!el) return;
    const conditions = detail.conditions || [];
    if (!conditions.length) {
      el.innerHTML = '';
      return;
    }
    el.innerHTML = conditions
      .map(
        (c) =>
          `<span class="condition-pill ${escapeHtml((c.status || '').toLowerCase())}">${escapeHtml(c.type)}: ${escapeHtml(c.status)}${c.reason ? ' · ' + escapeHtml(c.reason) : ''}</span>`
      )
      .join('');
  }

  function renderRolloutTimeline(detail) {
    const el = $('rollout-stage-timeline');
    if (!el) return;
    const r = detail.rollout || detail;
    const current =
      detail.rollout?.currentStage ??
      detail.rollout?.current_stage ??
      0;
    const phase = rolloutPhase(r, detail);
    const stages = detail.stages || [];
    el.innerHTML = stages
      .map((s) => {
        const result = s.resultPhase || s.result_phase;
        let cls = 'timeline-step';
        if (stageResultDone(result)) cls += ' done';
        else if (phase === 'failed' && (s.resultPhase || s.result_phase)) cls += ' failed';
        else if (s.index === current && rolloutIsActive(r?.phase)) cls += ' active';
        else if (s.index < current) cls += ' done';
        const dotContent = stageResultDone(result)
          ? '✓'
          : (result || '').toLowerCase() === 'failed'
            ? '✕'
            : s.index === current && rolloutIsActive(r?.phase)
              ? String(s.index + 1)
              : String(s.index + 1);
        return `<div class="${cls}" title="${escapeHtml(s.name)}"><span class="timeline-dot">${dotContent}</span><span class="timeline-label">${escapeHtml(s.name)}</span></div>`;
      })
      .join('');
  }

  function renderRolloutStages(detail) {
    const grid = $('rollout-stages-grid');
    if (!grid) return;
    grid.innerHTML = '';
    const r = detail.rollout || detail;
    const current =
      detail.rollout?.currentStage ??
      detail.rollout?.current_stage ??
      0;
    const awaiting =
      detail.rollout?.awaitingApproval ?? detail.rollout?.awaiting_approval;
    const total = rolloutStageTotal(r, detail) || 1;
    const pct = rolloutProgressPct(r, detail);
    const fill = $('rollout-progress-fill');
    if (fill) fill.style.width = pct + '%';
    const pctEl = $('rollout-progress-pct');
    if (pctEl) pctEl.textContent = pct + '%';
    const hint = $('rollout-pipeline-hint');
    if (hint) {
      const done = rolloutCompletedStages(r, detail);
      hint.textContent = `${done}/${total} stages complete`;
    }
    (detail.stages || []).forEach((s) => {
      const card = document.createElement('article');
      card.className = 'rollout-stage-card';
      const result = s.resultPhase || s.result_phase;
      if (stageResultDone(result)) card.classList.add('done');
      else if ((result || '').toLowerCase() === 'failed') card.classList.add('failed');
      else if (s.index === current) card.classList.add('current');
      if (s.index === current && awaiting) card.classList.add('awaiting');
      const stageType = s.stageType || s.stage_type || '';
      const ns = (s.namespaces || []).join(', ') || '—';
      const approval = s.requiresApproval || s.requires_approval;
      let resultLine = 'Pending';
      if (stageResultDone(result)) resultLine = 'Succeeded';
      else if ((result || '').toLowerCase() === 'failed') resultLine = 'Failed';
      else if (s.index === current) resultLine = awaiting ? 'Awaiting approval' : 'In progress';
      card.innerHTML = `
        <div class="rollout-stage-card-head">
          <h5>${escapeHtml(s.name)}</h5>
          <span class="rollout-stage-num">#${s.index + 1}</span>
        </div>
        <div class="rollout-stage-type"><span aria-hidden="true">${stageTypeIcon(stageType)}</span> ${escapeHtml(stageType)}${approval ? ' · manual' : ' · auto'}</div>
        <div class="rollout-stage-ns">${escapeHtml(ns)}</div>
        <div class="rollout-stage-result">${escapeHtml(resultLine)}${s.resultMessage || s.result_message ? ' — ' + escapeHtml(s.resultMessage || s.result_message) : ''}</div>
      `;
      grid.appendChild(card);
    });
    renderRolloutTimeline(detail);
    renderRolloutMeshTarget(detail);
    renderRolloutConditions(detail);
  }

  async function refreshRolloutDetail(r, quiet) {
    try {
      const res = await fetch(
        API() +
          `/api/v1/rollouts/${encodeURIComponent(r.namespace)}/${encodeURIComponent(r.name)}`
      );
      if (!res.ok) throw new Error(await res.text());
      rolloutDetail = await res.json();
      renderRolloutDetailHeader(r, rolloutDetail);
      renderRolloutStages(rolloutDetail);
      updateApproveAuthHint();
      await loadRolloutAudit(r.namespace, r.name);
      if (!quiet) setStatus(`Rollout ${r.namespace}/${r.name} loaded`);
      if (!quiet) startMigrationPolling();
    } catch (e) {
      if (!quiet) setStatus('Failed to load rollout: ' + e.message, true);
      renderRolloutStages({ stages: [] });
    }
  }

  function renderRolloutDetailHeader(r, detail) {
    setRolloutDetailVisible(true);
    const shortName = r.name || '—';
    $('rollout-detail-title').textContent = shortName;
    const phase = r.phase || detail?.rollout?.phase || '—';
    const meta = rolloutPhaseMeta(phase);
    const phaseEl = $('rollout-detail-phase');
    const iconEl = $('rollout-phase-icon');
    const chipEl = $('rollout-phase-chip');
    if (phaseEl) phaseEl.textContent = meta.label;
    if (iconEl) iconEl.textContent = meta.icon;
    if (chipEl) chipEl.className = 'rollout-phase-chip ' + meta.class;
    const hero = $('rollout-hero-card');
    if (hero) hero.className = 'rollout-hero-card phase-' + meta.class;
    const current = r.currentStage ?? r.current_stage ?? 0;
    const total = rolloutStageTotal(r, detail);
    const approved = (r.approvedStage ?? r.approved_stage ?? 0) + 1;
    const stageName =
      detail?.stages?.find((s) => s.index === current && rolloutIsActive(phase))?.name ||
      (meta.class === 'completed' ? 'All stages' : `Stage ${current + 1}`);
    const progressEl = $('rollout-stage-progress');
    if (progressEl) {
      if (meta.class === 'completed') {
        progressEl.textContent = `All ${total} stages complete · approved through stage ${approved}`;
      } else {
        progressEl.textContent = `${stageName} · ${rolloutStageLabel(r, detail)} · approved through ${approved}`;
      }
    }
    const cr = r.clusterRef || r.cluster_ref || detail?.rollout?.clusterRef || detail?.rollout?.cluster_ref;
    const crEl = $('rollout-cluster-ref');
    if (crEl) crEl.textContent = cr ? `Cluster ${cr}` : `Namespace ${r.namespace}`;
    const planRef = r.planRef || r.plan_ref;
    const planEl = $('rollout-plan-ref');
    if (planEl) planEl.textContent = planRef ? `Linked plan · ${planRef}` : 'Standalone rollout';
    const planLink = $('rollout-plan-link');
    if (planLink) {
      if (planRef) {
        planLink.classList.remove('hidden');
        planLink.textContent = `Open plan ${planRef}`;
      } else planLink.classList.add('hidden');
    }
    const autoRb = detail?.autoRollback ?? detail?.auto_rollback;
    if (autoRb !== undefined && progressEl) {
      progressEl.textContent += autoRb ? ' · auto-rollback enabled' : ' · auto-rollback off';
    }
    const awaiting =
      r.awaitingApproval ||
      r.awaiting_approval ||
      detail?.rollout?.awaitingApproval ||
      detail?.rollout?.awaiting_approval;
    const banner = $('rollout-awaiting-banner');
    if (banner) banner.classList.toggle('hidden', !awaiting);
    const approveBtn = $('approve-rollout');
    if (approveBtn) {
      approveBtn.classList.toggle('pulse', !!awaiting && canApproveRollout(awaiting));
      approveBtn.disabled = !canApproveRollout(awaiting);
    }
  }

  async function selectRollout(key) {
    selectedRolloutKey = key;
    const r = rollouts.find((x) => rolloutKey(x) === key);
    if (!r) return;
    renderRolloutList();
    renderRolloutDetailHeader(r, rolloutDetail);
    setStatus('Loading rollout detail…');
    await refreshRolloutDetail(r, false);
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
        el.classList.add('hidden');
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
          ? 'Single ambient mesh detected. '
          : ambient.length > 1
            ? 'Multiple ambient meshes — set rollout.spec.meshTarget. '
            : '') + lines.join(' · ');
      el.classList.remove('hidden');
    } catch (e) {
      el.classList.add('hidden');
    }
  }

  async function loadRollouts(quiet) {
    if (!quiet) setStatus('Loading rollouts…');
    loadMeshInstances();
    try {
      const res = await fetch(API() + '/api/v1/rollouts' + clusterQueryPrefix());
      if (!res.ok) throw new Error(await res.text());
      rollouts = await res.json();
      renderRolloutList();
      if (!rollouts.length) {
        setRolloutDetailVisible(false);
        if (!quiet) {
          setStatus('No rollouts yet — start one from a migration plan');
        }
        return;
      }
      if (!quiet) {
        setStatus(
          `Loaded ${rollouts.length} rollout(s)${activeClusterRef ? ' · ' + activeClusterRef : ''}`
        );
      }
      if (rollouts.length && !selectedRolloutKey) {
        await selectRollout(rolloutKey(rollouts[0]));
      } else {
        startMigrationPolling();
      }
    } catch (e) {
      if (!quiet) setStatus('Failed to load rollouts: ' + e.message, true);
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
      startDashboardPolling();
      if (!$('dashboard')?.classList.contains('hidden')) await loadDashboard(true);
    } catch (e) {
      setStatus('Approve failed: ' + e.message, true);
      $('approve-rollout').disabled = false;
    }
  }

  async function executeMigrationFromPlan() {
    const p = plans.find((x) => planKey(x) === selectedPlanKey);
    if (!p) return;
    const btn = $('execute-migration');
    if (btn) btn.disabled = true;
    setStatus('Approving plan and starting migration…');
    try {
      const res = await fetch(
        API() +
          `/api/v1/plans/${encodeURIComponent(p.namespace)}/${encodeURIComponent(p.name)}/execute`,
        {
          method: 'POST',
          headers: authHeaders({ 'Content-Type': 'application/json' }),
          body: getToken() ? '{}' : JSON.stringify({ actor: 'portal' }),
        }
      );
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      setStatus(data.message || `Migration started: ${data.rolloutName || data.rollout_name}`);
      p.approved = true;
      selectedRolloutKey =
        (data.rolloutNamespace || data.rollout_namespace) +
        '/' +
        (data.rolloutName || data.rollout_name);
      await loadPlans();
      await selectPlan(planKey(p));
      showPanel('rollouts');
      await loadRollouts();
      if (selectedRolloutKey) await selectRollout(selectedRolloutKey);
      startDashboardPolling();
      await loadDashboard(true);
    } catch (e) {
      setStatus('Execute failed: ' + e.message, true);
      if (btn) btn.disabled = false;
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
        if (id === 'dashboard') {
          loadDashboard();
          ensureMigrationPolling();
        }
        if (id === 'assessments') {
          loadAssessments();
          loadMeshInstancesForPlans();
        }
        if (id === 'plans') {
          loadPlans();
          ensureMigrationPolling();
        }
        if (id === 'rollouts') loadRollouts();
      });
    });
  }

  function initSse() {
    if (!API()) return;
    const evtSource = new EventSource(API() + '/api/v1/events/assessment');
    evtSource.onmessage = (e) => {
      try {
        const parsed = JSON.parse(e.data);
        if (parsed.channel === 'dashboard') {
          if (!$('dashboard')?.classList.contains('hidden')) loadDashboard(true);
          ensureMigrationPolling();
          return;
        }
      } catch (_) {}
      appendEvent(e.data);
    };
    evtSource.onerror = () => appendEvent('SSE connection error');
  }

  document.addEventListener('DOMContentLoaded', () => {
    consumeOidcTokenFromUrl();
    initNav();
    loadAuthConfig().then(() => {
      if (getToken()) setStatus('Signed in as ' + (parseJwtUsername(getToken()) || 'user'));
    });
    loadFleetClusters();
    $('cluster-select')?.addEventListener('change', (e) => onClusterChange(e.target.value));
    $('rollout-filter')?.addEventListener('input', renderRolloutList);
    $('plan-auto-refresh')?.addEventListener('change', startMigrationPolling);
    $('rollout-auto-refresh')?.addEventListener('change', startMigrationPolling);
    $('dash-auto-refresh')?.addEventListener('change', startMigrationPolling);
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
    $('plan-mesh-select')?.addEventListener('change', () => {
      updatePlanLabelPreview();
      updateMigrationSelectionUi();
    });
    $('app-select-page')?.addEventListener('click', selectEligibleOnPage);
    $('app-clear-selection')?.addEventListener('click', clearMigrationSelection);
    $('create-migration-plan')?.addEventListener('click', createMigrationPlan);
    $('refresh-plans')?.addEventListener('click', loadPlans);
    $('export-plan')?.addEventListener('click', downloadPlanExport);
    $('execute-migration')?.addEventListener('click', executeMigrationFromPlan);
    $('start-rollout')?.addEventListener('click', startRolloutFromPlan);
    $('refresh-rollouts')?.addEventListener('click', loadRollouts);
    $('approve-rollout')?.addEventListener('click', approveCurrentRolloutStage);
    initSse();
    showPanel('dashboard');
    loadDashboard();
    ensureMigrationPolling();
  });
})();
