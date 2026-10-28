import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation, useNavigate, useParams, useSearchParams } from "react-router";
import { ArrowLeft, TriangleAlert, LoaderCircle } from "lucide-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { runtime } from "@/runtime";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { StatusBadge } from "@/components/StatusBadge";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ModelSlotPicker } from "@/components/ModelSlotPicker";
import { ModelPoolPicker, poolTestKey } from "@/components/ModelPoolPicker";
import { SubscriptionBalancePanel } from "@/components/SubscriptionBalanceCard";
import { SubscriptionQuotaCard } from "@/components/SubscriptionQuotaCard";
import { api } from "@/api/tauri";
import { KeyValueEditor, type KeyValueRow } from "@/components/KeyValueEditor";
import { validateConnection, validateRequiredHeaders } from "@/lib/connectionValidation";
import { useProviders } from "@/hooks/useProviders";
import {
  useDeleteSubscription,
  useSetSubscriptionEnabled,
  useUpdateSubscription,
  useUpdateSubscriptionKey,
  useUpdateTokenQuotas,
} from "@/hooks/useSubscriptions";
import { useT } from "@/i18n";
import type {
  AuthHeaderFormat,
  ModelInfo,
  ModelPoolEntry,
  PoolTestItem,
  ModelSlots,
  QuotaPeriod,
  RefreshModelListResult,
  SlotEfforts,
  SubscriptionPatch,
  TestConnectionResult,
  TokenQuotas,
} from "@/types";
import { uniformSlots } from "@/lib/modelSlots";
import { lastErrorText, testResultText } from "@/lib/backendText";
import { providerName } from "@/lib/providerText";
import { UrlParamFields, urlParamErrors } from "@/components/UrlParamFields";

/** 池测试的遍历顺序 (与池总览的列顺序一致) */
const POOL_TEST_ORDER = ["vision", "llm", "multimodal", "speech", "embedding"];

/**
 * 测试失败后后端会把该模型 priority 挪到链尾并落库。把它同步回本地条目:
 * 先按 `能力分类\0模型名` 对齐, 用库里的新 priority 覆盖;
 * 本地新增但还没保存的行原样保留 (第一次保存时才入库), 不能整份覆盖掉。
 */
function mergeSavedPriorities(
  local: ModelPoolEntry[],
  saved: ModelPoolEntry[],
): ModelPoolEntry[] {
  const key = (e: ModelPoolEntry) => `${e.capability}\u0000${e.model_name}`;
  const savedMap = new Map(saved.map((e) => [key(e), e]));
  const localKeys = new Set(local.map(key));
  const out = local.map((l) => {
    const s = savedMap.get(key(l));
    return s ? { ...l, priority: s.priority } : l;
  });
  // 库里新增、本地没有的条目 (别处写入) 也带上, 避免下次保存把它们删掉
  for (const s of saved) {
    if (!localKeys.has(key(s))) out.push(s);
  }
  return out;
}
import { formatTokenShorthand, parseTokenShorthand } from "@/lib/quota";
import { errorText } from "@/lib/errorText";

export function SubscriptionEditPage() {
  const { t, locale } = useT();
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const location = useLocation();
  const [searchParams] = useSearchParams();
  // 池专属路由 /pool/subscriptions/:id; 兼容旧的 /subscriptions/:id?mode=pool
  const poolMode =
    location.pathname.startsWith("/pool/subscriptions/") ||
    searchParams.get("mode") === "pool";

  const subQuery = useQuery({
    queryKey: ["subscription", id],
    queryFn: () => api.getSubscription(id!),
    enabled: !!id,
  });
  const providers = useProviders();
  const updateMut = useUpdateSubscription();
  const updateQuotasMut = useUpdateTokenQuotas();
  const deleteMut = useDeleteSubscription();
  const enabledMut = useSetSubscriptionEnabled();
  const updateKeyMut = useUpdateSubscriptionKey();

  // 内置订阅: 在 providers 列表里反查 yaml 模板, 用于显示 endpoint 下拉选项
  const provider = useMemo(
    () =>
      subQuery.data && !subQuery.data.is_user_defined
        ? providers.data?.find((p) => p.id === subQuery.data!.provider_id)
        : undefined,
    [providers.data, subQuery.data],
  );

  const [endpointId, setEndpointId] = useState<string>("");
  const [urlParams, setUrlParams] = useState<Record<string, string>>({});
  const [showParamErrors, setShowParamErrors] = useState(false);
  const [displayName, setDisplayName] = useState<string>("");
  const [slots, setSlots] = useState<ModelSlots>(uniformSlots(""));
  const [slotEfforts, setSlotEfforts] = useState<SlotEfforts>({});
  const [quotaInputs, setQuotaInputs] = useState<Record<QuotaPeriod, string>>({
    daily: "",
    weekly: "",
    monthly: "",
    total: "",
  });
  const [quotaError, setQuotaError] = useState<string | null>(null);

  // 自定义订阅可编辑的连接字段
  const [baseUrl, setBaseUrl] = useState<string>("");
  const [messagesPath, setMessagesPath] = useState<string>("");
  const [authHeaderName, setAuthHeaderName] = useState<string>("");
  const [authHeaderFormat, setAuthHeaderFormat] = useState<AuthHeaderFormat>("bearer");
  const [providerDisplayName, setProviderDisplayName] = useState<string>("");
  const [requiredHeaders, setRequiredHeaders] = useState<KeyValueRow[]>([]);
  const [forwardClientHeaders, setForwardClientHeaders] = useState(false);
  /** 用户显式指定的模型列表完整 URL (如 DashScope 兼容端点) */
  const [modelsUrl, setModelsUrl] = useState<string>("");

  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);

  const [testResult, setTestResult] = useState<TestConnectionResult | null>(null);
  const [keyDialog, setKeyDialog] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [deleteDialog, setDeleteDialog] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  /** 池模式: 该订阅的能力链条目 (与订阅字段一起保存) */
  const [poolEntries, setPoolEntries] = useState<ModelPoolEntry[]>([]);
  const [poolSaving, setPoolSaving] = useState(false);
  /** 池模式逐模型测试: 结果键 = poolTestKey(capability, model), 页面与内嵌组件共用这一份 */
  const [poolTestResults, setPoolTestResults] = useState<Record<string, PoolTestItem>>({});
  /** 本次测试覆盖了哪些行 (用于统计启用/成功/失败) */
  const [poolTestMeta, setPoolTestMeta] = useState<
    { capability: string; model_name: string; enabled: boolean }[]
  >([]);
  const [poolTestingKey, setPoolTestingKey] = useState<string | null>(null);
  const [poolTestProgress, setPoolTestProgress] = useState<{ done: number; total: number } | null>(
    null,
  );
  const [poolTestRunning, setPoolTestRunning] = useState(false);
  /** ?test=1 自动测试只跑一次 */
  const didAutoTest = useRef(false);

  useEffect(() => {
    if (subQuery.data) {
      setEndpointId(subQuery.data.endpoint_id);
      setUrlParams(subQuery.data.url_params ?? {});
      setShowParamErrors(false);
      setDisplayName(subQuery.data.display_name);
      setSlots(subQuery.data.model_slots);
      setSlotEfforts(subQuery.data.slot_efforts ?? {});
      const q = subQuery.data.token_quotas ?? {};
      setQuotaInputs({
        daily: formatTokenShorthand(q.daily),
        weekly: formatTokenShorthand(q.weekly),
        monthly: formatTokenShorthand(q.monthly),
        total: formatTokenShorthand(q.total),
      });
      setQuotaError(null);
      setBaseUrl(subQuery.data.base_url);
      setMessagesPath(subQuery.data.messages_path);
      setAuthHeaderName(subQuery.data.auth_header_name);
      setAuthHeaderFormat(subQuery.data.auth_header_format);
      setProviderDisplayName(subQuery.data.provider_display_name);
      setForwardClientHeaders(subQuery.data.forward_client_headers ?? false);
      setRequiredHeaders(
        Object.entries(subQuery.data.required_headers ?? {}).map(([key, value]) => ({
          key,
          value,
        })),
      );
      setModelsUrl(subQuery.data.model_discovery.url ?? "");
      if (subQuery.data.model_cache) {
        setModels(subQuery.data.model_cache.models);
      }
    }
  }, [subQuery.data?.id]);

  // 池模式: 加载该订阅的能力链条目
  useEffect(() => {
    if (!poolMode || !id) return;
    let cancelled = false;
    api
      .loadModelPool(id)
      .then((entries) => {
        if (!cancelled) setPoolEntries(entries);
      })
      .catch(() => {
        if (!cancelled) setPoolEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [poolMode, id]);

  // 从池总览点「测试」进入 (?test=1) 时自动跑一轮按列表测试, 跑完清掉参数避免刷新重复
  useEffect(() => {
    if (!poolMode || !id) return;
    if (searchParams.get("test") !== "1") return;
    if (didAutoTest.current) return;
    didAutoTest.current = true;
    void runPoolTests(POOL_TEST_ORDER);
    navigate(`/pool/subscriptions/${id}`, { replace: true });
  }, [poolMode, id, searchParams, navigate]);

  if (!id) return null;

  async function refreshModels(): Promise<ModelInfo[] | null> {
    if (!id) return null;
    setFetchingModels(true);
    setModelError(null);
    try {
      const result: RefreshModelListResult = await runtime.invoke("refresh_model_list", { id });
      if (result.kind === "auto") {
        setModels(result.models);
        return result.models;
      }
      setModels(null);
      setModelError(result.reason);
      return null;
    } catch (e) {
      setModelError(errorText(e));
      return null;
    } finally {
      setFetchingModels(false);
    }
  }

  async function save() {
    if (!id || !sub) return;
    setSaveError(null);
    // 池模式不写四槽: 模型链走 model_pools 表, 与订阅字段分开存
    if (!poolMode && sub.is_user_defined && sub.endpoint_protocol === "systemone" && !slots.jev?.trim()) {
      return setSaveError(t("subscriptionNew.errFillJevModel"));
    }
    const patch: SubscriptionPatch = poolMode
      ? { display_name: displayName }
      : { display_name: displayName, model_slots: slots, slot_efforts: slotEfforts };
    // 仅透传订阅有此开关 (翻译类订阅不渲染也不提交, 保持后端字段不变)
    if (sub.auth_type === "api_key" && !isSystemOne) {
      patch.forward_client_headers = forwardClientHeaders;
    }
    if (sub.is_user_defined) {
      const connErrKey = validateConnection({ base_url: baseUrl, messages_path: messagesPath });
      if (connErrKey) return setSaveError(t(connErrKey));
      const headerRows = requiredHeaders
        .map((r) => ({ key: r.key.trim(), value: r.value.trim() }))
        .filter((r) => r.key !== "" || r.value !== ""); // 全空行静默丢弃
      const hdrErr = validateRequiredHeaders(headerRows);
      if (hdrErr) return setSaveError(t(hdrErr));
      patch.connection = {
        base_url: baseUrl.trim(),
        messages_path: messagesPath.trim(),
        auth_header_name: authHeaderName.trim(),
        auth_header_format: authHeaderFormat,
        provider_display_name: providerDisplayName.trim(),
        required_headers: Object.fromEntries(headerRows.map((r) => [r.key, r.value])),
        models_url: modelsUrl.trim(),
      };
    } else {
      // 内置订阅: 切 endpoint 走 endpoint_id patch (后端 re-snapshot)
      const selectedEndpoint = provider?.endpoints.find((e) => e.id === endpointId);
      if (provider && selectedEndpoint) {
        if (Object.keys(urlParamErrors(provider, selectedEndpoint, urlParams)).length > 0) {
          setShowParamErrors(true);
          return;
        }
        const currentParams = Object.fromEntries(
          selectedEndpoint.url_params_used.map((pid) => [pid, (urlParams[pid] ?? "").trim()]),
        );
        const paramsChanged = JSON.stringify(currentParams) !== JSON.stringify(sub.url_params ?? {});
        if (endpointId !== sub.endpoint_id) patch.endpoint_id = endpointId;
        if (paramsChanged || endpointId !== sub.endpoint_id) patch.url_params = currentParams;
      } else if (endpointId !== sub.endpoint_id) {
        patch.endpoint_id = endpointId;
      }
    }
    try {
      await updateMut.mutateAsync({ id, patch });
      // 池模式: 同一次保存顺带落能力链, 避免用户再跳去总览配一遍
      if (poolMode) {
        setPoolSaving(true);
        try {
          await api.saveModelPool(
            id,
            poolEntries.map(({ subscription_id: _sub, ...rest }) => rest),
          );
          await queryClient.invalidateQueries({ queryKey: ["model-pools"] });
        } finally {
          setPoolSaving(false);
        }
      }
    } catch (e) {
      setSaveError(`${t("subscriptionEdit.errSave")}: ${errorText(e)}`);
    }
  }

  async function saveQuotas() {
    if (!id) return;
    setQuotaError(null);
    const out: TokenQuotas = {};
    for (const p of ["daily", "weekly", "monthly", "total"] as QuotaPeriod[]) {
      const v = parseTokenShorthand(quotaInputs[p]);
      if (v === null) return setQuotaError(t("quota.invalid"));
      if (v !== undefined) out[p] = v;
    }
    try {
      await updateQuotasMut.mutateAsync({ id, quotas: out });
    } catch (e) {
      setQuotaError(errorText(e));
    }
  }

  async function testConnection() {
    if (!id) return;
    setTestResult(null);
    const result = await api.testConnection(id);
    setTestResult(result);
    if (result.ok && result.state_reset) {
      queryClient.invalidateQueries({ queryKey: ["subscriptions"] });
      queryClient.invalidateQueries({ queryKey: ["subscription", id] });
    }
  }

  /**
   * 池模式: 按能力链列表逐个模型测试 (指定能力分类; 传全部 5 个即"测试全部")。
   * 不走 test_connection —— 那个用四槽选出的"代表模型", 对池订阅可能测到
   * 根本不在池里、或 key 未开通的模型 (403 AccessDenied), 结论没参考价值。
   * 逐行调用 test_pool_model: 每测完一个就写一行, 颜色与统计立刻更新。
   * 结果存在页面级 state 里, 内嵌的 ModelPoolPicker 通过 externalTest 读同一份。
   */
  /**
   * 测试后同步顺序: 测试失败的模型后端已降序到链尾并落库, 这里把新 priority
   * 拉回本地 —— 否则用户点一次保存会用旧 priority 覆盖, 降序白做。
   */
  async function syncPoolPriorities() {
    if (!id) return;
    try {
      const saved = await api.loadModelPool(id);
      setPoolEntries((prev) => mergeSavedPriorities(prev, saved));
    } catch {
      // 同步失败不影响测试结果展示; 下次保存按旧顺序写入即可
    }
  }

  async function runPoolTests(caps: string[]) {
    if (!id || poolTestRunning) return;
    setPoolTestRunning(true);
    setPoolTestingKey(null);
    try {
      const saved = await api.loadModelPool(id);
      const queue = saved
        .filter((e) => caps.includes(e.capability))
        .slice()
        .sort(
          (a, b) =>
            POOL_TEST_ORDER.indexOf(a.capability) - POOL_TEST_ORDER.indexOf(b.capability) ||
            a.priority - b.priority,
        );
      setPoolTestMeta(
        queue.map((e) => ({
          capability: e.capability,
          model_name: e.model_name,
          enabled: e.enabled,
        })),
      );

      // 只重置本次覆盖的分类; 未启用的行立刻标灰跳过, 启用的留空等着测
      setPoolTestResults((prev) => {
        const next: Record<string, PoolTestItem> = {};
        for (const [k, v] of Object.entries(prev)) {
          if (!caps.includes(k.split("\u0000")[0])) next[k] = v;
        }
        for (const e of queue) {
          if (!e.enabled) {
            next[poolTestKey(e.capability, e.model_name)] = {
              model_name: e.model_name,
              priority: e.priority,
              enabled: false,
              ok: false,
              message: "未启用, 跳过测试",
            };
          }
        }
        return next;
      });

      const enabledRows = queue.filter((e) => e.enabled);
      setPoolTestProgress({ done: 0, total: enabledRows.length });

      for (let i = 0; i < enabledRows.length; i++) {
        const e = enabledRows[i];
        const key = poolTestKey(e.capability, e.model_name);
        setPoolTestingKey(key);
        let item: PoolTestItem;
        try {
          item = await api.testPoolModel(id, e.capability, e.model_name);
        } catch (err) {
          item = {
            model_name: e.model_name,
            priority: e.priority,
            enabled: true,
            ok: false,
            message: errorText(err),
          };
        }
        // 测完一个就落一个, 统计数字逐个增长
        setPoolTestResults((prev) => ({ ...prev, [key]: item }));
        setPoolTestProgress({ done: i + 1, total: enabledRows.length });
      }

      // 失败的模型后端已经降序到链尾 (写库了)。这里把新 priority 同步回本地,
      // 否则用户随后点一次保存会用旧 priority 覆盖, 降序白做 —— 而且编辑器里
      // 列表按 priority 排序, 同步后失败的模型会立刻沉到底部。
      await syncPoolPriorities();
    } finally {
      setPoolTestingKey(null);
      setPoolTestRunning(false);
    }
  }

  async function confirmDelete() {
    if (!id) return;
    await deleteMut.mutateAsync(id);
    navigate(poolMode ? "/pool/subscriptions" : "/subscriptions");
  }

  async function confirmUpdateKey() {
    if (!id || !newKey) return;
    await updateKeyMut.mutateAsync({ id, newKey });
    setKeyDialog(false);
    setNewKey("");
  }

  const sub = subQuery.data;
  const isSystemOne = sub?.endpoint_protocol === "systemone";

  if (subQuery.isLoading) {
    return <div className="p-8 text-sm text-muted-foreground">{t("common.loading")}</div>;
  }

  if (!sub) {
    return <div className="p-8 text-sm text-muted-foreground">{t("subscriptionEdit.notFound")}</div>;
  }

  const isCustom = sub.is_user_defined;
  const selectedEndpointForParams = provider?.endpoints.find((e) => e.id === endpointId);

  return (
    <>
      <div className="page-bar">
        <div className="page-col page-bar-lead">
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8 shrink-0"
            asChild
            title={t("subscriptionNew.backToList")}
            aria-label={t("subscriptionNew.backToList")}
          >
            <Link to={poolMode ? "/pool/subscriptions" : "/subscriptions"}>
              <ArrowLeft className="h-4 w-4" />
            </Link>
          </Button>
          <h1 title={sub.display_name}>{sub.display_name}</h1>
          <StatusBadge state={sub.state} className="shrink-0" />
          {isCustom && (
            <span className="shrink-0 text-xs px-2 py-0.5 rounded bg-muted text-muted-foreground">
              🔧 {t("subscriptions.custom")}
            </span>
          )}
        </div>
      </div>

      <div className="page-flow">
        <div className="page-flow-pad">
          <div className="page-col space-y-6">
            {(sub.referenced_by.length > 0 || sub.last_error_message) && (
              <div className="space-y-1">
                {sub.referenced_by.length > 0 && (
                  <div className="text-sm text-muted-foreground">
                    {t("subscriptionEdit.referencedPrefix")}{sub.referenced_by.join(", ")}
                  </div>
                )}
                {sub.last_error_message && (
                  <div className="text-xs text-destructive">
                    {lastErrorText(sub.last_error_code, sub.last_error_message, t)}
                  </div>
                )}
              </div>
            )}

            <Card>
              <CardHeader>
                <CardTitle>{t("subscriptionEdit.basicInfo")}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                  <Label>{t("subscriptionNew.field.provider")}</Label>
                  {isCustom ? (
                    <Input
                      value={providerDisplayName}
                      onChange={(e) => setProviderDisplayName(e.target.value)}
                      placeholder={t("subscriptionEdit.providerNamePh")}
                    />
                  ) : (
                    <div className="flex items-center gap-2 text-sm text-muted-foreground">
                      <ProviderIcon iconId={sub.provider_icon} size={18} />
                      <span>{providerName(sub, locale)}{t("subscriptionEdit.providerLocked")}</span>
                    </div>
                  )}
                </div>

                {/* 内置订阅: endpoint 切换下拉 */}
                {!isCustom && (
                  <div className="grid grid-cols-[120px_1fr] gap-3 items-start">
                    <Label className="mt-2">{t("subscriptionNew.field.endpoint")}</Label>
                    <div className="space-y-1 min-w-0">
                      <Select value={endpointId} onValueChange={setEndpointId}>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {provider?.endpoints.filter((e) => e.protocol === sub.endpoint_protocol).map((e) => (
                            <SelectItem key={e.id} value={e.id} subtitle={e.base_url}>
                              {e.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <div className="font-mono text-[10px] text-muted-foreground break-all">
                        {sub.base_url}
                        {sub.messages_path}
                      </div>
                      {endpointId !== sub.endpoint_id && (
                        <div className="text-xs text-amber-600">
                          {t("subscriptionEdit.endpointChangeWarn")}
                        </div>
                      )}
                    </div>
                  </div>
                )}

                {!isCustom && provider && selectedEndpointForParams && (
                  <UrlParamFields
                    provider={provider}
                    endpoint={selectedEndpointForParams}
                    values={urlParams}
                    onChange={(pid, v) => setUrlParams((prev) => ({ ...prev, [pid]: v }))}
                    showErrors={showParamErrors}
                    layout="grid"
                  />
                )}

                {/* 自定义订阅: base_url / messages_path / auth 可编辑 */}
                {isCustom && (
                  <>
                    <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                      <Label>Base URL</Label>
                      <Input
                        className="font-mono"
                        value={baseUrl}
                        onChange={(e) => setBaseUrl(e.target.value)}
                        placeholder="https://api.example.com"
                      />
                    </div>
                    <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                      <Label>Messages Path</Label>
                      <Input
                        className="font-mono"
                        value={messagesPath}
                        onChange={(e) => setMessagesPath(e.target.value)}
                        placeholder="/v1/messages"
                      />
                    </div>
                    <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                      <Label>{t("subscriptionEdit.authHeaderName")}</Label>
                      <Input
                        className="font-mono"
                        value={authHeaderName}
                        onChange={(e) => setAuthHeaderName(e.target.value)}
                        placeholder={t("subscriptionEdit.authHeaderNamePh")}
                      />
                    </div>
                    <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                      <Label>{t("subscriptionEdit.authHeaderFormat")}</Label>
                      <Select
                        value={authHeaderFormat}
                        onValueChange={(v) => setAuthHeaderFormat(v as AuthHeaderFormat)}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="bearer">{t("subscriptionEdit.authBearerOption")}</SelectItem>
                          <SelectItem value="raw">{t("subscriptionEdit.authRawOption")}</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="grid grid-cols-[120px_1fr] gap-3 items-start">
                      <Label className="mt-2">{t("subscriptionEdit.requiredHeaders")}</Label>
                      <div className="space-y-1">
                        <KeyValueEditor
                          value={requiredHeaders}
                          onChange={setRequiredHeaders}
                          keyPlaceholder={t("subscriptionEdit.requiredHeaderKeyPh")}
                          valuePlaceholder={t("subscriptionEdit.requiredHeaderValuePh")}
                          addLabel={t("subscriptionEdit.requiredHeadersAdd")}
                          removeAriaLabel={t("subscriptionEdit.requiredHeadersRemove")}
                        />
                        <p className="text-xs text-muted-foreground">
                          {t("subscriptionEdit.requiredHeadersHint")}
                        </p>
                      </div>
                    </div>
                    <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                      <Label>模型列表 URL</Label>
                      <div className="flex items-center gap-2">
                        <Input
                          className="font-mono flex-1"
                          value={modelsUrl}
                          onChange={(e) => setModelsUrl(e.target.value)}
                          placeholder="留空让后端自动探测, 如 https://dashscope.aliyuncs.com/compatible-mode/v1/models"
                        />
                        {modelsUrl && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setModelsUrl("")}
                            title="清空让后端自动 probe"
                          >
                            清空
                          </Button>
                        )}
                      </div>
                      <p className="col-start-2 text-xs text-muted-foreground -mt-2">
                        仅当自动探测找不到模型时才需要填 (如 DashScope 自定义网关没有 /models 端点)。
                        保存后下次刷新模型列表会优先用这个地址。
                      </p>
                    </div>
                  </>
                )}

                <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                  <Label>{t("subscriptionNew.field.note")}</Label>
                  <Input value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
                </div>
                <div className="grid grid-cols-[120px_1fr] gap-3 items-center">
                  <Label>API Key</Label>
                  <div className="flex items-center gap-2">
                    <Input type="password" value="••••••••••••••" disabled />
                    <Button variant="outline" size="sm" onClick={() => setKeyDialog(true)}>
                      {t("common.modify")}
                    </Button>
                  </div>
                </div>

                {/* 仅 Anthropic 透传订阅: 透传客户端请求头开关 (翻译类协议不消费此字段) */}
                {sub.auth_type === "api_key" && !isSystemOne && (
                  <div className="grid grid-cols-[120px_1fr] gap-3 items-start">
                    <Label className="mt-0.5">{t("subscriptionEdit.forwardClientHeaders")}</Label>
                    <div className="space-y-1">
                      <Switch
                        checked={forwardClientHeaders}
                        onCheckedChange={setForwardClientHeaders}
                      />
                      <p className="text-xs text-muted-foreground">
                        {t("subscriptionEdit.forwardClientHeadersHint")}
                      </p>
                    </div>
                  </div>
                )}
              </CardContent>
            </Card>

            {!poolMode && (
            <Card>
              <CardHeader>
                <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12, width: "100%" }}>
                  <CardTitle>{t("subscriptionEdit.modelSlotsTitle")}</CardTitle>
                </div>
              </CardHeader>
              <CardContent>
                <ModelSlotPicker
                  value={slots}
                  onChange={setSlots}
                  efforts={slotEfforts}
                  onEffortsChange={setSlotEfforts}
                  effortDisabled={sub.auth_type === "kiro_oauth"}
                  effortDisabledReason={t("slotEffort.unsupportedKiro")}
                  models={models}
                  loading={fetchingModels}
                  error={modelError}
                  onRefresh={refreshModels}
                  exampleModels={
                    provider?.endpoints.find((e) => e.id === endpointId)?.example_models ?? sub.model_discovery.example_models
                  }
                  systemone={isSystemOne}
                  jevRequired={isSystemOne && sub.is_user_defined}
                />
                {sub.model_cache && !isSystemOne && (
                  <div className="mt-3 text-xs text-muted-foreground">
                    {t("subscriptionEdit.modelCacheUpdated")}{new Date(sub.model_cache.fetched_at).toLocaleString()}
                  </div>
                )}
                {!isSystemOne && (
                  <div className="mt-3 text-xs text-muted-foreground" style={{ lineHeight: 1.6 }}>
                    {t("subscriptionEdit.poolMovedHint")}{" "}
                    <Link to="/pool">{t("sidebar.nav.pool")}</Link>
                  </div>
                )}
              </CardContent>
            </Card>
            )}
            {poolMode && (
              <Card>
                <CardHeader>
                  <CardTitle>{t("pool.editChain")}</CardTitle>
                </CardHeader>
                <CardContent>
                  <ModelPoolPicker
                    subscriptionId={id!}
                    entries={poolEntries}
                    onChange={setPoolEntries}
                    models={models}
                    loading={fetchingModels}
                    onRefresh={refreshModels}
                    externalTest={{
                      results: poolTestResults,
                      testingKey: poolTestingKey,
                      running: poolTestRunning,
                      progress: poolTestProgress,
                      run: (cap) => void runPoolTests([cap]),
                    }}
                  />
                </CardContent>
              </Card>
            )}

            <SubscriptionBalancePanel
              subscription={sub}
              onChanged={() => {
                queryClient.invalidateQueries({ queryKey: ["subscription", id] });
                queryClient.invalidateQueries({ queryKey: ["subscriptions"] });
              }}
            />

            <Card>
              <CardHeader>
                <CardTitle>{t("quota.title")}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                <p className="text-sm text-muted-foreground">{t("quota.desc")}</p>
                {(["daily", "weekly", "monthly", "total"] as QuotaPeriod[]).map((p) => (
                  <div key={p} className="flex items-center gap-3">
                    <Label className="w-24 shrink-0">{t(`quota.period.${p}`)}</Label>
                    <Input
                      className="font-mono"
                      placeholder={t("quota.placeholder")}
                      value={quotaInputs[p]}
                      onChange={(e) => setQuotaInputs({ ...quotaInputs, [p]: e.target.value })}
                    />
                  </div>
                ))}
                {quotaError && <p className="text-sm text-destructive">{quotaError}</p>}
                <Button size="sm" onClick={saveQuotas} disabled={updateQuotasMut.isPending}>
                  {updateQuotasMut.isPending && <LoaderCircle className="h-4 w-4 animate-spin" />}
                  {t("quota.save")}
                </Button>
              </CardContent>
            </Card>

            <SubscriptionQuotaCard
              subscription={sub}
              onChanged={() => {
                queryClient.invalidateQueries({ queryKey: ["subscription", id] });
                queryClient.invalidateQueries({ queryKey: ["subscriptions"] });
              }}
            />
          </div>
        </div>
      </div>

      {/* 底部常驻操作区: 与顶栏对称地挂在 .page-flow 之外, 不随内容滚动。
          反馈紧挨着触发它的按钮 —— 结果若跟着内容滚动, 用户在内容区底部点保存就看不见。 */}
      <div className="page-foot">
        {(testResult || saveError || (poolMode && poolTestMeta.length > 0)) && (
          <div className="page-foot-feedback">
            <div className="page-col space-y-2">
              {testResult && (
                <Alert variant={testResult.ok ? "default" : "destructive"}>
                  <AlertDescription>
                    <div>{testResultText(testResult, t)}</div>
                    {testResult.model_used && (
                      <div className="mt-1 text-xs opacity-80">
                        {t("subscriptionEdit.testModel")}<code className="font-mono">{testResult.model_used}</code>
                      </div>
                    )}
                    {testResult.state_reset && (
                      <div className="mt-1 text-xs">{t("subscriptionEdit.stateReset")}</div>
                    )}
                  </AlertDescription>
                </Alert>
              )}
              {saveError && (
                <Alert variant="destructive">
                  <AlertDescription>{saveError}</AlertDescription>
                </Alert>
              )}
              {poolMode && poolTestMeta.length > 0 && (() => {
                const enabledMeta = poolTestMeta.filter((m) => m.enabled);
                const items = enabledMeta
                  .map((m) => poolTestResults[poolTestKey(m.capability, m.model_name)])
                  .filter((i): i is PoolTestItem => !!i);
                const okN = items.filter((i) => i.ok).length;
                const fails = enabledMeta
                  .map((m) => ({ meta: m, item: poolTestResults[poolTestKey(m.capability, m.model_name)] }))
                  .filter((x): x is { meta: (typeof enabledMeta)[number]; item: PoolTestItem } =>
                    !!x.item && !x.item.ok,
                  );
                const pending = enabledMeta.length - items.length;
                return (
                  <Alert variant={fails.length === 0 ? "default" : "destructive"}>
                    <AlertDescription>
                      <div>
                        {poolTestRunning
                          ? `正在按列表测试 ${items.length}/${enabledMeta.length}…  已成功 ${okN} · 已失败 ${fails.length}`
                          : `按列表测试完成: 启用 ${enabledMeta.length} 个 · 成功 ${okN} · 失败 ${fails.length}`}
                        {pending > 0 && !poolTestRunning && ` · 未测 ${pending}`}
                      </div>
                      {fails.length > 0 && (
                        <div className="mt-1 text-xs">
                          失败:{" "}
                          {fails
                            .map(
                              (f) =>
                                `${f.meta.model_name} (${f.meta.capability}${
                                  f.item.http_status ? ` · HTTP ${f.item.http_status}` : ""
                                })`,
                            )
                            .join("、")}
                        </div>
                      )}
                      {fails.length > 0 && !poolTestRunning && (
                        <div className="mt-1 text-xs">
                          失败的模型已自动排到各自能力链的队尾, 下次请求不会再先撞它。
                        </div>
                      )}
                    </AlertDescription>
                  </Alert>
                );
              })()}
            </div>
          </div>
        )}
        <div className="page-foot-bar">
          <div className="page-col page-foot-inner">
            <Label className="text-sm">{t("subscriptionEdit.enabled")}</Label>
            <Switch
              checked={sub.enabled}
              onCheckedChange={(checked) =>
                enabledMut.mutate({ id: sub.id, enabled: checked })
              }
            />
            <div className="page-foot-actions">
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  poolMode ? void runPoolTests(POOL_TEST_ORDER) : void testConnection()
                }
                disabled={poolTestRunning}
              >
                {poolTestRunning
                  ? `测试中 ${poolTestProgress?.done ?? 0}/${poolTestProgress?.total ?? 0}…`
                  : poolMode
                    ? "按列表测试全部模型"
                    : t("subscriptionEdit.testConn")}
              </Button>
              <Button variant="destructive" size="sm" onClick={() => setDeleteDialog(true)}>
                {t("common.delete")}
              </Button>
              <Button size="sm" onClick={save} disabled={updateMut.isPending || poolSaving}>
                {(updateMut.isPending || poolSaving) && <LoaderCircle className="h-4 w-4 animate-spin" />}
                {t("common.save")}
              </Button>
            </div>
          </div>
        </div>
      </div>

      <Dialog open={keyDialog} onOpenChange={setKeyDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("subscriptionEdit.keyDialog.title")}</DialogTitle>
            <DialogDescription>{t("subscriptionEdit.keyDialog.desc")}</DialogDescription>
          </DialogHeader>
          <Input
            type="password"
            value={newKey}
            onChange={(e) => setNewKey(e.target.value)}
            placeholder={t("subscriptionEdit.keyDialog.placeholder")}
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setKeyDialog(false)}>
              {t("common.cancel")}
            </Button>
            <Button onClick={confirmUpdateKey} disabled={!newKey}>
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteDialog} onOpenChange={setDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              <div className="flex items-center gap-2">
                <TriangleAlert className="h-4 w-4 text-destructive" />
                {t("subscriptionEdit.deleteDialog.titlePrefix")}{sub.display_name}{t("subscriptionEdit.deleteDialog.titleSuffix")}
              </div>
            </DialogTitle>
            <DialogDescription>
              {sub.referenced_by.length > 0 ? (
                <>
                  {t("subscriptionEdit.deleteDialog.refByPrefix")}{sub.referenced_by.length}{t("subscriptionEdit.deleteDialog.refBySuffix")}
                  <ul className="mt-2 list-disc pl-5">
                    {sub.referenced_by.map((name) => (
                      <li key={name}>{name}</li>
                    ))}
                  </ul>
                  <p className="mt-2">{t("subscriptionEdit.deleteDialog.refByOutro")}</p>
                </>
              ) : (
                t("subscriptionEdit.deleteDialog.noRef")
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteDialog(false)}>
              {t("common.cancel")}
            </Button>
            <Button variant="destructive" onClick={confirmDelete}>
              {t("subscriptionEdit.deleteDialog.confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
