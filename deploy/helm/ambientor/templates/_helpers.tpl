{{- define "ambientor.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "ambientor.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "ambientor.namespace" -}}
{{- .Values.namespace }}
{{- end }}

{{- define "ambientor.labels" -}}
app.kubernetes.io/name: {{ include "ambientor.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{- end }}

{{- define "ambientor.auth.secretName" -}}
{{- .Values.auth.existingSecret | default .Values.auth.secretName | default "ambientor-secrets" -}}
{{- end }}

{{- define "ambientor.databaseUrlEnv" -}}
{{- if .Values.database.existingSecret.name -}}
- name: DATABASE_URL
  valueFrom:
    secretKeyRef:
      name: {{ .Values.database.existingSecret.name | quote }}
      key: {{ .Values.database.existingSecret.key | default "database-url" | quote }}
{{- else if include "ambientor.databaseUrl" . | trim -}}
- name: DATABASE_URL
  value: {{ include "ambientor.databaseUrl" . | trim | quote }}
{{- end -}}
{{- end }}

{{- define "ambientor.apiOidcEnv" -}}
{{- if .Values.auth.oidc.enabled -}}
- name: AMBIENTOR_OIDC_ISSUER_URL
  value: {{ required "auth.oidc.issuerUrl required when auth.oidc.enabled" .Values.auth.oidc.issuerUrl | quote }}
- name: AMBIENTOR_OIDC_CLIENT_ID
  value: {{ required "auth.oidc.clientId required when auth.oidc.enabled" .Values.auth.oidc.clientId | quote }}
- name: AMBIENTOR_OIDC_REDIRECT_URI
  value: {{ required "auth.oidc.redirectUri required when auth.oidc.enabled" .Values.auth.oidc.redirectUri | quote }}
{{- if or .Values.auth.createSecret .Values.auth.existingSecret }}
- name: AMBIENTOR_OIDC_CLIENT_SECRET
  valueFrom:
    secretKeyRef:
      name: {{ include "ambientor.auth.secretName" . }}
      key: oidc-client-secret
{{- else if .Values.auth.oidc.clientSecret }}
- name: AMBIENTOR_OIDC_CLIENT_SECRET
  value: {{ .Values.auth.oidc.clientSecret | quote }}
{{- else }}
{{- fail "auth.oidc.clientSecret (or auth.createSecret / auth.existingSecret with oidc-client-secret key) required when auth.oidc.enabled" }}
{{- end }}
{{- with .Values.auth.oidc.scopes }}
- name: AMBIENTOR_OIDC_SCOPES
  value: {{ . | quote }}
{{- end }}
{{- with .Values.auth.oidc.defaultRoles }}
- name: AMBIENTOR_OIDC_DEFAULT_ROLES
  value: {{ . | quote }}
{{- end }}
{{- with .Values.auth.oidc.successUrl }}
- name: AMBIENTOR_OIDC_SUCCESS_URL
  value: {{ . | quote }}
{{- end }}
{{- end -}}
{{- end }}

{{- define "ambientor.apiJwtEnv" -}}
{{- if or .Values.auth.createSecret .Values.auth.existingSecret }}
- name: AMBIENTOR_JWT_SECRET
  valueFrom:
    secretKeyRef:
      name: {{ include "ambientor.auth.secretName" . }}
      key: jwt-secret
{{- else if .Values.api.env.AMBIENTOR_JWT_SECRET }}
- name: AMBIENTOR_JWT_SECRET
  value: {{ .Values.api.env.AMBIENTOR_JWT_SECRET | quote }}
{{- end -}}
{{- end }}

{{- define "ambientor.webApiUrl" -}}
{{- if .Values.web.apiUrl -}}
{{- .Values.web.apiUrl -}}
{{- else if .Values.openshift.apiUrl -}}
{{- .Values.openshift.apiUrl -}}
{{- else if and .Values.ingress.enabled .Values.ingress.api.host -}}
{{- $scheme := ternary "https" "http" .Values.ingress.tls.enabled -}}
{{- printf "%s://%s" $scheme .Values.ingress.api.host -}}
{{- else -}}
{{- .Values.web.env.AMBIENTOR_API_URL -}}
{{- end -}}
{{- end }}

{{- define "ambientor.imageRef" -}}
{{- $root := required "root" .root -}}
{{- $component := required "component" .component -}}
{{- $defaultRegistry := $root.Values.image.registry -}}
{{- $defaultTag := $root.Values.image.tag -}}
{{- $componentValues := (get $root.Values $component | default dict) -}}
{{- $componentImage := (get $componentValues "image" | default dict) -}}
{{- $repo := (get $componentImage "repository" | default "") -}}
{{- $tag := (get $componentImage "tag" | default $defaultTag) -}}
{{- if $repo -}}
{{- printf "%s:%s" $repo $tag -}}
{{- else if $defaultRegistry -}}
{{- printf "%s/ambientor-%s:%s" $defaultRegistry $component $tag -}}
{{- else -}}
{{- printf "ambientor-%s:%s" $component $tag -}}
{{- end -}}
{{- end }}

{{- define "ambientor.databaseUrl" -}}
{{- if .Values.database.url -}}
{{- .Values.database.url -}}
{{- else if .Values.postgresql.enabled -}}
{{- printf "postgres://%s:%s@%s-postgresql:5432/%s" .Values.postgresql.auth.username .Values.postgresql.auth.password (include "ambientor.fullname" .) .Values.postgresql.auth.database -}}
{{- end -}}
{{- end }}

{{- define "ambientor.postgresql.stsName" -}}
{{- printf "%s-postgresql" (include "ambientor.fullname" .) -}}
{{- end }}

{{/*
  "true" when the chart should render volumeClaimTemplates (not emptyDir).
  Reuses existing PVC-backed StatefulSets on upgrade (immutable volumeClaimTemplates).
*/}}
{{- define "ambientor.postgresql.useVolumeClaimTemplates" -}}
{{- $sts := lookup "apps/v1" "StatefulSet" .Values.namespace (include "ambientor.postgresql.stsName" .) -}}
{{- if $sts -}}
{{- if $sts.spec.volumeClaimTemplates -}}
{{- print "true" -}}
{{- else -}}
{{- print "false" -}}
{{- end -}}
{{- else if dig "enabled" true .Values.postgresql.primary.persistence -}}
{{- print "true" -}}
{{- else -}}
{{- print "false" -}}
{{- end -}}
{{- end }}
