{{/* Names and labels. */}}
{{- define "bicraft-controller.fullname" -}}
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

{{/* The controller's pods carry app.kubernetes.io/name: bicraft-controller: workspace pods admit
these pods only (controllerPodLabels in the configuration). */}}
{{- define "bicraft-controller.selectorLabels" -}}
app.kubernetes.io/name: bicraft-controller
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "bicraft-controller.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "bicraft-controller.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: controller
{{- end -}}

{{- define "bicraft-controller.image" -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- if .Values.image.digest -}}
{{- printf "%s:%s@%s" .Values.image.repository $tag .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}
{{- end -}}

{{- define "bicraft-controller.authSecretName" -}}
{{- printf "%s-auth" (include "bicraft-controller.fullname" .) -}}
{{- end -}}

{{/* The Host values the gateway answers: the Service's names, the ingress and route hosts and
auth.allowedHosts (the public URL's host is added by the controller itself). */}}
{{- define "bicraft-controller.allowedHosts" -}}
{{- $svc := include "bicraft-controller.fullname" . -}}
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

{{/* The controller's configuration file (crates/controller/src/config.rs). */}}
{{- define "bicraft-controller.config" -}}
{{- $w := .Values.workspaces -}}
{{- $secrets := list -}}
{{- range .Values.imagePullSecrets -}}
{{- $secrets = append $secrets .name -}}
{{- end -}}
{{- $pod := dict "shutdownGraceSeconds" (int $w.pod.shutdownGraceSeconds) "maxUploadBytes" (toString $w.pod.maxUploadBytes) "allowedOrigins" $w.pod.allowedOrigins "extraArgs" $w.pod.extraArgs "extraEnv" $w.pod.extraEnv "nodeSelector" $w.pod.nodeSelector "tolerations" $w.pod.tolerations "priorityClassName" $w.pod.priorityClassName "imagePullSecrets" $secrets "annotations" $w.pod.annotations "runAsUser" (int .Values.podSecurityContext.runAsUser) "runAsGroup" (int .Values.podSecurityContext.runAsGroup) "fsGroup" (int .Values.podSecurityContext.fsGroup) "maxViewsPerUser" (int $w.pod.maxViewsPerUser) "maxViews" (int $w.pod.maxViews) "viewIdleSeconds" (int $w.pod.viewIdleSeconds) "allowAutosave" $w.pod.allowAutosave "localOffset" (toString $w.pod.localOffset) -}}
{{- with $w.pod.viewerControl.existingSecret -}}
{{- $_ := set $pod "viewerControl" (dict "name" . "key" $w.pod.viewerControl.key) -}}
{{- end -}}
{{- $recovery := dict "enabled" $w.recovery.enabled "volumeSize" (toString $w.recovery.volumeSize) "storageClass" $w.recovery.storageClass -}}
{{- $labels := dict "app.kubernetes.io/name" "bicraft-controller" "app.kubernetes.io/instance" .Release.Name -}}
{{- $cfg := dict "crdGroup" .Values.crd.group "namespace" .Release.Namespace "image" (include "bicraft-controller.image" .) "imagePullPolicy" .Values.image.pullPolicy "classes" $w.classes "defaultClass" $w.defaultClass "defaultIdleTimeoutSeconds" (int $w.defaultIdleTimeoutSeconds) "defaultReclaimPolicy" $w.defaultReclaimPolicy "storageClass" $w.storageClass "egressProfiles" $w.egressProfiles "dnsNamespaceSelector" .Values.networkPolicy.dns.namespaceSelector "dnsPodSelector" .Values.networkPolicy.dns.podSelector "controllerPodLabels" $labels "networkPolicy" $w.networkPolicy "gatewayTrustedCidrs" $w.gatewayTrustedCidrs "wakeTimeoutSeconds" (int $w.wakeTimeoutSeconds) "publicUrl" .Values.auth.publicUrl "pod" $pod "recovery" $recovery -}}
{{- toJson $cfg -}}
{{- end -}}

{{/* Refuse configurations the controller would refuse at start. */}}
{{- define "bicraft-controller.validate" -}}
{{- $a := .Values.auth -}}
{{- $tokens := or $a.apiTokens.existingSecret $a.apiTokens.tokensJson -}}
{{- $config := or $a.config.existingSecret $a.config.json -}}
{{- if not (or $tokens $config $a.oidc.issuers $a.trustedProxy.cidrs) -}}
{{- fail "the gateway listens on every interface of the pod and needs authentication: set auth.apiTokens, auth.oidc.issuers, auth.trustedProxy.cidrs or auth.config (docs/auth.md)" -}}
{{- end -}}
{{- if and (not .Values.controller.admins) (not $a.grants) (not $a.readerGrants) (not $config) -}}
{{- fail "set controller.admins (who creates workspaces), auth.grants or auth.readerGrants" -}}
{{- end -}}
{{- if not (hasKey .Values.workspaces.classes .Values.workspaces.defaultClass) -}}
{{- fail "workspaces.defaultClass must name one of workspaces.classes" -}}
{{- end -}}
{{- if not (regexMatch "^[a-z0-9]([-a-z0-9]*[a-z0-9])?([.][a-z0-9]([-a-z0-9]*[a-z0-9])?)+$" .Values.crd.group) -}}
{{- fail "crd.group must be a DNS name with a dot (e.g. bicraft.example.com)" -}}
{{- end -}}
{{- end -}}
