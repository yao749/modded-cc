import { useCallback } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/tauri";
import { useT } from "@/i18n";
import { localizeProvider } from "@/lib/providerText";
import type { ProviderInfo } from "@/types";

/** 厂商列表, 上屏文字已按当前界面语言换好 (切语言时 select 重算, 不重新请求)。 */
export function useProviders() {
  const { locale } = useT();
  const select = useCallback(
    (data: ProviderInfo[]) => data.map((p) => localizeProvider(p, locale)),
    [locale],
  );
  return useQuery({
    queryKey: ["providers"],
    queryFn: () => api.listProviders(),
    staleTime: Infinity,
    select,
  });
}
