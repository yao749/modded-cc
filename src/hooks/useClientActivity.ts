import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/tauri";

export const CLIENT_ACTIVITY_KEY = ["client-activity"] as const;

/**
 * Live Routing 的被动流量检测: 哪些客户端最近真的把请求发给了 cc-router。
 * 10s 轮询与订阅列表同频; RouteFlowDiagram 与「客户端接入」卡共用此 key,
 * React Query 去重后同一轮只打一次后端。
 */
export function useClientActivity() {
  return useQuery({
    queryKey: CLIENT_ACTIVITY_KEY,
    queryFn: () => api.getClientActivity(),
    refetchInterval: 10_000,
    staleTime: 10_000,
  });
}
