{{/* Names and labels. */}}
{{- define "bicraft.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "bicraft.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "bicraft.selectorLabels" -}}
app.kubernetes.io/name: {{ include "bicraft.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "bicraft.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "bicraft.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: mcp-server
bicraft.dev/workspace: {{ .Values.workspace.id | quote }}
{{- end -}}

{{- define "bicraft.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- if .Values.image.digest -}}
{{- printf "%s:%s@%s" .Values.image.repository $tag .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}
{{- end -}}

{{- define "bicraft.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "bicraft.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "bicraft.claimName" -}}
{{- default (printf "%s-workspace" (include "bicraft.fullname" .)) .Values.persistence.existingClaim -}}
{{- end -}}

{{- define "bicraft.recoveryClaimName" -}}
{{- default (printf "%s-recovery" (include "bicraft.fullname" .)) .Values.recovery.persistence.existingClaim -}}
{{- end -}}

{{- define "bicraft.authSecretName" -}}
{{- printf "%s-auth" (include "bicraft.fullname" .) -}}
{{- end -}}

{{/* The pod memory limit in MiB: BICRAFT_MEMORY_LIMIT + 64 MiB (docs/beyond-ram.md, lot 5). */}}
{{- define "bicraft.podMemoryMiB" -}}
{{- $limit := int .Values.model.memoryLimitMiB -}}
{{- if gt $limit 0 -}}
{{- add $limit 64 -}}
{{- else -}}
{{- 0 -}}
{{- end -}}
{{- end -}}

{{- define "bicraft.scanThreads" -}}
{{- if gt (int .Values.model.scanThreads) 0 -}}
{{- int .Values.model.scanThreads -}}
{{- else -}}
{{- int .Values.model.cpu -}}
{{- end -}}
{{- end -}}

{{- define "bicraft.aggThreads" -}}
{{- if gt (int .Values.model.aggThreads) 0 -}}
{{- int .Values.model.aggThreads -}}
{{- else -}}
{{- max 1 (div (int .Values.model.cpu) 4) -}}
{{- end -}}
{{- end -}}

{{/* The Host values the server answers: the Service's names, the ingress and route hosts, the
public URL's host (added by the server itself) and auth.allowedHosts. */}}
{{- define "bicraft.allowedHosts" -}}
{{- $svc := include "bicraft.fullname" . -}}
{{- $ns := .Release.Namespace -}}
{{- $port := toString .Values.service.port -}}
{{- $hosts := list -}}
{{- range $n := list $svc (printf "%s.%s" $svc $ns) (printf "%s.%s.svc" $svc $ns) (printf "%s.%s.svc.cluster.local" $svc $ns) -}}
{{- $hosts = append $hosts $n -}}
{{- $hosts = append $hosts (printf "%s:%s" $n $port) -}}
{{- end -}}
{{- if .Values.ingress.enabled -}}
{{- range .Values.ingress.hosts -}}
{{- $hosts = append $hosts . -}}
{{- end -}}
{{- end -}}
{{- if .Values.httpRoute.enabled -}}
{{- range .Values.httpRoute.hostnames -}}
{{- $hosts = append $hosts . -}}
{{- end -}}
{{- end -}}
{{- range .Values.auth.allowedHosts -}}
{{- $hosts = append $hosts . -}}
{{- end -}}
{{- join "," (uniq $hosts) -}}
{{- end -}}

{{/* Refuse configurations `bicraft serve` would refuse at start, before anything is created. */}}
{{- define "bicraft.validate" -}}
{{- if not (regexMatch "^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$" .Values.workspace.id) -}}
{{- fail "workspace.id must be 1 to 64 of [A-Za-z0-9._-] and must not start with a dot" -}}
{{- end -}}
{{- $a := .Values.auth -}}
{{- $tokens := or $a.apiTokens.existingSecret $a.apiTokens.tokensJson -}}
{{- $config := or $a.config.existingSecret $a.config.json -}}
{{- if not (or $tokens $config $a.oidc.issuers $a.trustedProxy.cidrs) -}}
{{- fail "bicraft listens on every interface of the pod and needs authentication: set auth.apiTokens, auth.oidc.issuers, auth.trustedProxy.cidrs or auth.config (docs/auth.md)" -}}
{{- end -}}
{{- if and (not $a.grants) (not $a.readerGrants) (not $config) -}}
{{- fail "auth.grants and auth.readerGrants are empty: without a grant every workspace is closed (e.g. token:ci=*)" -}}
{{- end -}}
{{- if and .Values.databricks.enabled (not .Values.databricks.existingSecret) -}}
{{- fail "databricks.enabled needs databricks.existingSecret (the token's Secret)" -}}
{{- end -}}
{{- if and .Values.viewer.control.enabled (not .Values.viewer.enabled) -}}
{{- fail "viewer.control.enabled needs the viewer (viewer.enabled)" -}}
{{- end -}}
{{- if and .Values.viewer.control.enabled (not .Values.viewer.control.existingSecret) -}}
{{- fail "viewer.control.enabled needs viewer.control.existingSecret (the control token's Secret, at least 32 characters)" -}}
{{- end -}}
{{- if and .Values.recovery.enabled (not (hasPrefix "/" (toString .Values.recovery.mountPath))) -}}
{{- fail "recovery.mountPath must be an absolute path" -}}
{{- end -}}
{{- if and .Values.recovery.enabled (hasPrefix (printf "%s/" (trimSuffix "/" .Values.workspace.root)) (printf "%s/" (trimSuffix "/" .Values.recovery.mountPath))) -}}
{{- fail "recovery.mountPath must be outside workspace.root (journals hold users' edits; workspace files are listed and downloaded)" -}}
{{- end -}}
{{- if lt (int .Values.model.cpu) 1 -}}
{{- fail "model.cpu must be a whole number of cores, at least 1" -}}
{{- end -}}
{{- end -}}
