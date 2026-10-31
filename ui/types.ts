export interface Metrics {
  successful: number; last_saved: string | null; saved_run: number; pending: number;
  active: number; retry: number; failed: number; persistence: number;
  sitemaps_pending: number; sitemaps_retry: number; sitemaps_active: number;
  sitemaps_done: number; sitemaps_failed: number; seen: number; discovered: number; next_due: string | null;
}
export interface Snapshot {
  analysis: import('./analysis').AnalysisStatus;
  state: string; detail: string; metrics: Metrics; events: string[];
  active: boolean; busy: boolean; closing: boolean; storage_ok: boolean; selected: boolean;
  storage_checking: boolean; catalogue_loaded: boolean;
  elapsed_secs: number; downloads: number; parsers: number; workers: number;
  website_delay_ms: number | null; api_delay_ms: number | null; cooldown_secs: number;
  reprocessed: number; reprocess_failed: number; outdated: number; can_resume: boolean;
  auth_method: 'anonymous' | 'manual' | 'browser'; browser_name: string | null;
  has_manual_cookies: boolean; remembered: boolean; browser_container: number;
  root: string; version: string; catalogue_version: number; record_version: number; parser_version: string;
}
export interface Profile { id: number; browser: string; name: string; containers: [number, string][] }
export interface Draft {
  method: 'anonymous' | 'manual' | 'browser'; profile_id: number | null; container: number;
  website_cookie: string; api_cookie: string; keep_saved: boolean; remember: boolean; product_id: string | null;
}
export interface AccessResult { passed: boolean; text: string }
export interface FailurePage { rows: string[]; next: number | null }
export type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
