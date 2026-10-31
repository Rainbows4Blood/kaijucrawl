-- Keep request scheduling independent of the size of the pending backlog.
CREATE INDEX jobs_ready ON jobs(run,kind,due_at,id)
 WHERE state IN ('pending','retry');
CREATE INDEX jobs_attempt_budget ON jobs(run,attempts)
 WHERE state IN ('pending','retry');
CREATE INDEX pending_product_intents ON intents(target,product_id) WHERE NOT finalized;
-- Dashboard reads use narrow covering indexes rather than job/product payloads.
CREATE INDEX jobs_run_counts ON jobs(run,kind,state);
CREATE INDEX jobs_discovered ON jobs(discovered_run) WHERE kind='product';
CREATE INDEX products_saved ON products(saved_at);
