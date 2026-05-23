(function () {
  const API = () => window.AMBIENTOR_API_URL || '';

  const $ = (id) => document.getElementById(id);

  let assessments = [];
  let selectedKey = null;
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

  function renderScores(scores, prefix) {
    $(prefix + 'overall-score').textContent = scores?.overall ?? '—';
    const readiness = $('dash-readiness');
    const sidecar = $('dash-sidecar');
    const traffic = $('dash-traffic');
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

  function itemKey(a) {
    return a.namespace + '/' + a.name;
  }

  function renderAssessmentList() {
    const ul = $('assessment-list');
    ul.innerHTML = '';
    assessments.forEach((a) => {
      const li = document.createElement('li');
      const key = itemKey(a);
      li.className = 'assessment-item' + (key === selectedKey ? ' selected' : '');
      li.innerHTML = `
        <button type="button" data-key="${escapeHtml(key)}">
          <span class="name">${escapeHtml(a.namespace)}/${escapeHtml(a.name)}</span>
          <span class="phase">${escapeHtml(a.phase)}</span>
          <span class="score-mini">${a.scores?.overall ?? '—'}/100</span>
        </button>
      `;
      li.querySelector('button').addEventListener('click', () => selectAssessment(key));
      ul.appendChild(li);
    });
  }

  function selectAssessment(key) {
    selectedKey = key;
    const a = assessments.find((x) => itemKey(x) === key);
    if (!a) return;
    renderAssessmentList();
    $('detail-title').textContent = `${a.namespace}/${a.name}`;
    $('detail-phase').textContent = a.phase;
    $('detail-phase').className = 'phase-badge ' + (a.phase || '').toLowerCase();
    renderScores(a.scores, 'detail-');
    renderSummary(a.summary, 'detail-');
    renderFindings(a.findings, 'detail-findings');
    showPanel('assessments');
  }

  async function loadAssessments() {
    setStatus('Loading assessments…');
    try {
      const res = await fetch(API() + '/api/v1/assessments');
      if (!res.ok) throw new Error(await res.text());
      assessments = await res.json();
      renderAssessmentList();
      setStatus(
        assessments.length
          ? `Loaded ${assessments.length} assessment(s)`
          : 'No completed assessments in cluster'
      );
      if (assessments.length && !selectedKey) {
        selectAssessment(itemKey(assessments[0]));
      }
    } catch (e) {
      setStatus('Failed to load assessments: ' + e.message, true);
    }
  }

  async function runAssessment() {
    setStatus('Running assessment…');
    $('run-assess').disabled = true;
    try {
      const res = await fetch(API() + '/api/v1/assess', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      renderScores(data.scores, 'dash-');
      renderSummary(data.summary, 'dash-');
      renderFindings(data.findings, 'dash-findings');
      setStatus(`Assessment complete (${(data.findings || []).length} findings)`);
      await loadAssessments();
      showPanel('dashboard');
    } catch (e) {
      setStatus('Assessment failed: ' + e.message, true);
    } finally {
      $('run-assess').disabled = false;
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
    $('approve-rollout').disabled = !awaiting;
    setStatus('Loading rollout detail…');
    try {
      const res = await fetch(
        API() + `/api/v1/rollouts/${encodeURIComponent(r.namespace)}/${encodeURIComponent(r.name)}`
      );
      if (!res.ok) throw new Error(await res.text());
      rolloutDetail = await res.json();
      renderRolloutStages(rolloutDetail);
      const awaitingDetail = rolloutDetail.rollout?.awaitingApproval ?? rolloutDetail.rollout?.awaiting_approval;
      $('approve-rollout').disabled = !awaitingDetail;
      await loadRolloutAudit(r.namespace, r.name);
      setStatus(`Rollout ${r.namespace}/${r.name} loaded`);
    } catch (e) {
      setStatus('Failed to load rollout: ' + e.message, true);
      renderRolloutStages({ stages: [] });
    }
    showPanel('rollouts');
  }

  async function loadRollouts() {
    setStatus('Loading rollouts…');
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
    $('approve-rollout').disabled = true;
    setStatus('Approving stage…');
    try {
      const res = await fetch(
        API() +
          `/api/v1/rollouts/${encodeURIComponent(r.namespace)}/${encodeURIComponent(r.name)}/approve`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ actor: 'portal' }),
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
    initNav();
    $('run-assess')?.addEventListener('click', runAssessment);
    $('refresh-assessments')?.addEventListener('click', loadAssessments);
    $('refresh-plans')?.addEventListener('click', loadPlans);
    $('export-plan')?.addEventListener('click', downloadPlanExport);
    $('start-rollout')?.addEventListener('click', startRolloutFromPlan);
    $('refresh-rollouts')?.addEventListener('click', loadRollouts);
    $('approve-rollout')?.addEventListener('click', approveCurrentRolloutStage);
    initSse();
    showPanel('dashboard');
  });
})();
