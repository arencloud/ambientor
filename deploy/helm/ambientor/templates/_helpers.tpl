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
{{- $registry := $root.Values.image.registry -}}
{{- $tag := $root.Values.image.tag -}}
{{- if $registry -}}
{{- printf "%s/ambientor-%s:%s" $registry $component $tag -}}
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
