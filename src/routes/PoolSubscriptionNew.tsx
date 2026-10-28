import { useState, useMemo } from "react";
import { useNavigate } from "react-router";
import { ArrowLeft, ArrowRight, LoaderCircle } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { ProviderLogo } from "@/components/ProviderLogo";
import { ProviderCapabilityTags } from "@/components/ProviderCapabilityTags";
import { UrlParamFields, urlParamErrors } from "@/components/UrlParamFields";
import { Spinner } from "@/components/Spinner";
import { ModelPoolPicker } from "@/components/ModelPoolPicker";
import { ChatGptOAuthDialog } from "@/components/ChatGptOAuthDialog";
import { KiroAuthDialog, type KiroAuthSuccessPayload } from "@/components/KiroAuthDialog";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/api/tauri";
import { useProviders } from "@/hooks/useProviders";
import { useT } from "@/i18n";
import { uniformSlots } from "@/lib/modelSlots";
import { errorText } from "@/lib/errorText";
import { runtime } from "@/runtime";
import { providerCapabilities } from "@/lib/providerCapabilities";
import type {
  AuthHeaderFormat,
  CreateSubscriptionInput,
  ModelInfo,
  ModelPoolEntry,
  ModelSlots,
  RefreshModelListResult,
} from "@/types";

type Step = 1 | 2;

const CUSTOM_VALUE = "custom";
const AUTH_PRESETS: Record<"bearer" | "x_api_key", { name: string; format: AuthHeaderFormat; label: string }> = {
  bearer: { name: "Authorization", format: "bearer", label: "Bearer Token" },
  x_api_key: { name: "X-API-Key", format: "raw", label: "X-API-Key" },
};

/**
 * 池专属的「添加订阅」页面。两步:
 *   步骤 1 接入 — 选厂商 / OAuth / 自定义表单 → 占位 create_subscription(routing_group="pool")
 *   步骤 2 能力配置 — ModelPoolPicker → save_model_pool → 回 /pool
 */
export function PoolSubscriptionNewPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const providers = useProviders();

  const [step, setStep] = useState<Step>(1);
  const [providerId, setProviderId] = useState<string>("");
  const [endpointId, setEndpointId] = useState<string>("");
  const [urlParams, setUrlParams] = useState<Record<string, string>>({});
  const [showParamErrors, setShowParamErrors] = useState(false);
  const [apiKey, setApiKey] = useState<string>("");
  const [displayName, setDisplayName] = useState<string>("");

  // 自定义路径字段
  const [customBaseUrl, setCustomBaseUrl] = useState<string>("");
  const [customMessagesPath, setCustomMessagesPath] = useState<string>("/v1/messages");
  const [customAuthPreset, setCustomAuthPreset] = useState<"bearer" | "x_api_key">("bearer");

  // OAuth state
  const [oauthDialogOpen, setOauthDialogOpen] = useState(false);
  const [oauthResult, setOauthResult] = useState<{
    deviceCode: string;
    account: { email?: string; account_id: string };
  } | null>(null);
  const [kiroDialogOpen, setKiroDialogOpen] = useState(false);
  const [kiroPayload, setKiroPayload] = useState<KiroAuthSuccessPayload | null>(null);

  // 步骤 1 结果
  const [createdId, setCreatedId] = useState<string>("");
  const [step1Models, setStep1Models] = useState<ModelInfo[] | null>(null);
  const [step1Error, setStep1Error] = useState<string | null>(null);

  // 步骤 2 state
  const [poolDraft, setPoolDraft] = useState<ModelPoolEntry[]>([]);
  const [poolModelsLoading, setPoolModelsLoading] = useState(false);

  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const provider = useMemo(
    () => providers.data?.find((p) => p.id === providerId),
    [providers.data, providerId],
  );
  const endpoint = useMemo(
    () => provider?.endpoints.find((e) => e.id === endpointId),
    [provider, endpointId],
  );

  const isCustom = providerId === CUSTOM_VALUE;
  const isChatGptOAuth = provider?.auth.type === "chatgpt_oauth";
  const isKiroOAuth = provider?.auth.type === "kiro_oauth";
  const oauthReady = isChatGptOAuth ? !!oauthResult : isKiroOAuth ? !!kiroPayload : !!apiKey;

  // 厂商分组 (沿用 SubscriptionNew.tsx 的 category 过滤方式)
  const firstParty = useMemo(
    () => (providers.data ?? []).filter((p) => (p.category ?? "first_party") === "first_party"),
    [providers.data],
  );
  const secondParty = useMemo(
    () => (providers.data ?? []).filter((p) => p.category === "second_party"),
    [providers.data],
  );
  const aggregators = useMemo(
    () => (providers.data ?? []).filter((p) => p.category === "aggregator"),
    [providers.data],
  );

  function handleProviderChange(id: string) {
    setProviderId(id);
    setEndpointId("");
    setOauthResult(null);
    setKiroPayload(null);
    setApiKey("");
    setDisplayName("");
    setUrlParams({});
  }

  function handleEndpointChange(id: string) {
    setEndpointId(id);
    setUrlParams({});
  }

  // 步骤 1 下一步 — 创建订阅 + 刷新模型列表
  async function goToStep2() {
    if (!providerId) return;
    if (!isCustom && !endpointId) return;
    if (!displayName) return;
    if (!oauthReady) return;
    if (!isCustom && provider && endpoint) {
      const errs = urlParamErrors(provider, endpoint, urlParams);
      if (Object.keys(errs).length > 0) {
        setShowParamErrors(true);
        return;
      }
    }
    if (isCustom && !customBaseUrl.trim()) {
      setStep1Error(t("subscriptionNew.errFillBase"));
      return;
    }
    setSubmitting(true);
    setStep1Error(null);

    try {
      const placeholderSlots: ModelSlots = uniformSlots("(pending)");
      const commonFields = {
        display_name: displayName,
        model_slots: placeholderSlots,
        routing_group: "pool" as const,
      };

      let created: { id: string };

      if (isChatGptOAuth && oauthResult && provider && endpoint) {
        created = await api.createChatGptOAuthSubscription({
          device_code: oauthResult.deviceCode,
          provider_id: provider.id,
          endpoint_id: endpoint.id,
          ...commonFields,
        });
      } else if (isKiroOAuth && kiroPayload && provider && endpoint) {
        created = await api.createKiroSubscription({
          provider_id: provider.id,
          endpoint_id: endpoint.id,
          session_id: kiroPayload.sessionId,
          device_code: kiroPayload.deviceCode,
          disguise: kiroPayload.disguise,
          profile_arn_override: kiroPayload.profileArn,
          ...commonFields,
        });
      } else if (isCustom) {
        const input: CreateSubscriptionInput = {
          display_name: displayName,
          api_key: apiKey,
          model_slots: placeholderSlots,
          source: {
            kind: "custom",
            provider_display_name: customBaseUrl,
            base_url: customBaseUrl.trim(),
            messages_path: customMessagesPath.trim(),
            auth_header_name: AUTH_PRESETS[customAuthPreset].name,
            auth_header_format: AUTH_PRESETS[customAuthPreset].format,
          },
          routing_group: "pool",
        };
        created = await api.createSubscription(input);
      } else if (provider && endpoint) {
        const input: CreateSubscriptionInput = {
          display_name: displayName,
          api_key: apiKey,
          model_slots: placeholderSlots,
          source: {
            kind: "from_template",
            provider_id: provider.id,
            endpoint_id: endpoint.id,
            url_params: Object.fromEntries(
              endpoint.url_params_used.map((id) => [id, (urlParams[id] ?? "").trim()]),
            ),
          },
          routing_group: "pool",
        };
        created = await api.createSubscription(input);
      } else {
        return; // 防御性
      }

      await queryClient.invalidateQueries({ queryKey: ["subscriptions"] });
      setCreatedId(created.id);

      // 刷新模型列表供步骤 2 ModelPoolPicker 自动填充
      setPoolModelsLoading(true);
      try {
        const result: RefreshModelListResult = await runtime.invoke("refresh_model_list", {
          id: created.id,
        });
        if (result.kind === "auto") {
          setStep1Models(result.models);
        } else {
          setStep1Models(null);
          setStep1Error(result.reason);
        }
      } catch (e) {
        setStep1Error(errorText(e));
        setStep1Models(null);
      } finally {
        setPoolModelsLoading(false);
      }

      setStep(2);
    } catch (e) {
      setStep1Error(t("subscriptionNew.errCreate") + ": " + errorText(e));
    } finally {
      setSubmitting(false);
    }
  }

  async function refreshPoolModels() {
    if (!createdId) return;
    setPoolModelsLoading(true);
    try {
      const result = await api.refreshModelList(createdId);
      if (result.kind === "auto") setStep1Models(result.models);
    } catch {
      // 忽略
    } finally {
      setPoolModelsLoading(false);
    }
  }

  async function savePool() {
    if (!createdId) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      await api.saveModelPool(
        createdId,
        poolDraft.map(({ subscription_id: _sub, ...rest }) => rest),
      );
      await queryClient.invalidateQueries({ queryKey: ["model-pools"] });
      navigate("/pool", { replace: true });
    } catch (e) {
      setSubmitError(errorText(e));
    } finally {
      setSubmitting(false);
    }
  }

  async function skipPool() {
    navigate("/pool", { replace: true });
  }

  function backToStep1() {
    setStep(1);
  }

  // 默认显示名
  const defaultDisplayName = provider
    ? endpoint
      ? `${provider.display_name} · ${endpoint.label}`
      : provider.display_name
    : t("pool.new.defaultName");

  return (
    <div className="page-container">
      <div className="page-header">
        <Button variant="ghost" size="sm" onClick={() => navigate("/pool")}>
          <ArrowLeft className="h-4 w-4" />
          {t("common.back")}
        </Button>
        <h1>{t("pool.new.title")}</h1>
        <div className="subtitle">
          {step === 1 ? t("pool.new.step1Hint") : t("pool.new.step2Hint")}
        </div>
      </div>

      {/* 步骤 1: 接入 */}
      {step === 1 && (
        <div className="card section">
          <div className="card-header">
            <h2>{t("pool.new.step1")}</h2>
          </div>
          <div className="card-body" style={{ display: "flex", flexDirection: "column", gap: 20 }}>
            {/* 厂商选择 */}
            <div className="field">
              <Label>{t("subscriptionNew.provider")}</Label>
              <Select value={providerId} onValueChange={handleProviderChange}>
                <SelectTrigger>
                  <SelectValue placeholder={t("subscriptionNew.selectProvider")} />
                </SelectTrigger>
                <SelectContent>
                  {firstParty.length > 0 && (
                    <SelectGroup>
                      <SelectLabel>{t("subscriptionNew.group.firstParty")}</SelectLabel>
                      {firstParty.map((p) => (
                        <SelectItem key={p.id} value={p.id}>
                          <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                            <ProviderLogo iconId={p.icon} size={16} />
                            {p.display_name}
                            <ProviderCapabilityTags caps={providerCapabilities(p)} />
                          </span>
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  )}
                  {secondParty.length > 0 && (
                    <SelectGroup>
                      <SelectLabel>{t("subscriptionNew.group.secondParty")}</SelectLabel>
                      {secondParty.map((p) => (
                        <SelectItem key={p.id} value={p.id}>
                          <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                            <ProviderLogo iconId={p.icon} size={16} />
                            {p.display_name}
                            <ProviderCapabilityTags caps={providerCapabilities(p)} />
                          </span>
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  )}
                  {aggregators.length > 0 && (
                    <SelectGroup>
                      <SelectLabel>{t("subscriptionNew.group.aggregator")}</SelectLabel>
                      {aggregators.map((p) => (
                        <SelectItem key={p.id} value={p.id}>
                          <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                            <ProviderLogo iconId={p.icon} size={16} />
                            {p.display_name}
                          </span>
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  )}
                  <SelectGroup>
                    <SelectLabel>{t("subscriptionNew.group.custom")}</SelectLabel>
                    <SelectItem value={CUSTOM_VALUE}>{t("subscriptionNew.customProvider")}</SelectItem>
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>

            {/* 端点选择 (非自定义) */}
            {provider && !isCustom && provider.endpoints.length > 0 && (
              <div className="field">
                <Label>{t("subscriptionNew.endpoint")}</Label>
                <Select value={endpointId} onValueChange={handleEndpointChange}>
                  <SelectTrigger>
                    <SelectValue placeholder={t("subscriptionNew.selectEndpoint")} />
                  </SelectTrigger>
                  <SelectContent>
                    {provider.endpoints.map((ep) => (
                      <SelectItem key={ep.id} value={ep.id}>
                        {ep.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* 自定义协议表单 */}
            {isCustom && (
              <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                <div className="field">
                  <Label>{t("subscriptionNew.customBase")}</Label>
                  <Input
                    value={customBaseUrl}
                    onChange={(e) => setCustomBaseUrl(e.target.value)}
                    placeholder="https://api.example.com"
                  />
                </div>
                <div className="field">
                  <Label>{t("subscriptionNew.customPath")}</Label>
                  <Input
                    value={customMessagesPath}
                    onChange={(e) => setCustomMessagesPath(e.target.value)}
                  />
                </div>
                <div className="field">
                  <Label>{t("subscriptionNew.customAuth")}</Label>
                  <Select value={customAuthPreset} onValueChange={(v) => setCustomAuthPreset(v as any)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="bearer">Authorization: Bearer</SelectItem>
                      <SelectItem value="x_api_key">X-API-Key: raw</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
            )}

            {/* URL 参数 (模板 endpoint 需要) */}
            {provider && !isCustom && endpoint && (
              <UrlParamFields
                provider={provider}
                endpoint={endpoint}
                values={urlParams}
                onChange={(id, v) => setUrlParams((prev) => ({ ...prev, [id]: v }))}
                showErrors={showParamErrors}
              />
            )}

            {/* API Key / OAuth */}
            {isChatGptOAuth ? (
              <div className="field">
                <Label>{t("subscriptionNew.apiKey")}</Label>
                {oauthResult ? (
                  <div className="oauth-connected">
                    <span className="oauth-icon">✓</span>
                    <div>
                      <strong>{t("oauth.chatgpt.connected")}</strong>
                      <div className="muted small">
                        {oauthResult.account.email ?? "-"} · {oauthResult.account.account_id}
                      </div>
                    </div>
                    <Button variant="ghost" size="sm" onClick={() => setOauthDialogOpen(true)}>
                      {t("oauth.chatgpt.reconnect")}
                    </Button>
                  </div>
                ) : (
                  <Button variant="outline" onClick={() => setOauthDialogOpen(true)}>
                    {t("oauth.chatgpt.connectButton")}
                  </Button>
                )}
              </div>
            ) : isKiroOAuth ? (
              <div className="field">
                <Label>{t("subscriptionNew.apiKey")}</Label>
                {kiroPayload ? (
                  <div className="oauth-connected">
                    <span className="oauth-icon">✓</span>
                    <div>
                      <strong>{t("oauth.kiro.connected")}</strong>
                      <div className="muted small">
                        {kiroPayload.authMethod} · {kiroPayload.region}
                      </div>
                    </div>
                    <Button variant="ghost" size="sm" onClick={() => setKiroDialogOpen(true)}>
                      {t("oauth.kiro.reconnect")}
                    </Button>
                  </div>
                ) : (
                  <Button variant="outline" onClick={() => setKiroDialogOpen(true)}>
                    {t("oauth.kiro.connectButton")}
                  </Button>
                )}
              </div>
            ) : (
              <div className="field">
                <Label>{t("subscriptionNew.apiKey")}</Label>
                <Input
                  type="password"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder="sk-..."
                />
              </div>
            )}

            {/* 显示名 */}
            <div className="field">
              <Label>{t("subscriptionNew.displayName")}</Label>
              <Input
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder={defaultDisplayName}
              />
            </div>

            {step1Error && (
              <div className="err-msg">⚠ {step1Error}</div>
            )}

            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 8 }}>
              <Button variant="outline" onClick={() => navigate("/pool")}>
                {t("common.cancel")}
              </Button>
              <Button
                onClick={() => void goToStep2()}
                disabled={submitting || !providerId || (!isCustom && !endpointId) || !displayName || !oauthReady}
              >
                {submitting ? (
                  <Spinner size={14} />
                ) : (
                  <>
                    {t("common.next")}
                    <ArrowRight className="h-4 w-4" />
                  </>
                )}
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* 步骤 2: 能力配置 */}
      {step === 2 && (
        <div className="card section">
          <div className="card-header">
            <h2>{t("pool.new.step2")}</h2>
          </div>
          <div className="card-body">
            {poolModelsLoading && (
              <div style={{ padding: 20, textAlign: "center" }}>
                <LoaderCircle className="h-5 w-5 animate-spin" />
              </div>
            )}
            {createdId && !poolModelsLoading && (
              <ModelPoolPicker
                subscriptionId={createdId}
                entries={poolDraft}
                onChange={setPoolDraft}
                models={step1Models}
                onRefresh={() => void refreshPoolModels()}
              />
            )}

            {step1Error && (
              <div className="err-msg" style={{ marginTop: 12 }}>⚠ {step1Error}</div>
            )}
            {submitError && (
              <div className="err-msg" style={{ marginTop: 12 }}>⚠ {submitError}</div>
            )}

            <div style={{ display: "flex", justifyContent: "space-between", marginTop: 16 }}>
              <Button variant="ghost" onClick={backToStep1} disabled={submitting}>
                <ArrowLeft className="h-4 w-4" />
                {t("common.back")}
              </Button>
              <div style={{ display: "flex", gap: 8 }}>
                <Button variant="outline" onClick={() => void skipPool()} disabled={submitting}>
                  {t("pool.new.skip")}
                </Button>
                <Button onClick={() => void savePool()} disabled={submitting}>
                  {submitting ? <LoaderCircle className="h-4 w-4 animate-spin" /> : null}
                  {t("common.done")}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* OAuth 对话框 — 用 onClose 不是 onOpenChange */}
      <ChatGptOAuthDialog
        open={oauthDialogOpen}
        onClose={() => setOauthDialogOpen(false)}
        onSuccess={(deviceCode, account) => {
          setOauthResult({ deviceCode, account });
          setOauthDialogOpen(false);
        }}
      />
      <KiroAuthDialog
        open={kiroDialogOpen}
        onClose={() => setKiroDialogOpen(false)}
        onSuccess={(payload) => {
          setKiroPayload(payload);
          setKiroDialogOpen(false);
        }}
      />
    </div>
  );
}
