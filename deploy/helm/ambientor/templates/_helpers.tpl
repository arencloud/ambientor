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
