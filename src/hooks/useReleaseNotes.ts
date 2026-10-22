import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/tauri";

export const RELEASE_NOTES_KEY = ["release-notes"] as const;

/** 内嵌内容在进程内不变; 「已读」的变化靠 mark 成功后与 settings_changed 事件 invalidate */
export function useReleaseNotes() {
  return useQuery({
    queryKey: RELEASE_NOTES_KEY,
    queryFn: () => api.getReleaseNotes(),
    staleTime: Infinity,
  });
}

export function useMarkReleaseNotesSeen() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => api.markReleaseNotesSeen(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: RELEASE_NOTES_KEY });
    },
  });
}
