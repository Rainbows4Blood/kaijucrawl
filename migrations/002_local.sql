CREATE TABLE runs (
 id BLOB PRIMARY KEY, kind text NOT NULL, settings TEXT NOT NULL,
 started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), ended_at TEXT,
 state text NOT NULL DEFAULT 'running', checkpoint text NOT NULL DEFAULT '',
 processed INTEGER NOT NULL DEFAULT 0, failures INTEGER NOT NULL DEFAULT 0,
 summary TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE targets (
 target text PRIMARY KEY, cooldown_until TEXT NOT NULL DEFAULT '1970-01-01T00:00:00.000Z',
 state text NOT NULL DEFAULT 'ready', reason text
);
CREATE TABLE jobs (
 id INTEGER PRIMARY KEY AUTOINCREMENT, kind text NOT NULL CHECK(kind IN ('product','sitemap')),
 target text NOT NULL REFERENCES targets(target), identity text NOT NULL, url text NOT NULL,
 run BLOB REFERENCES runs(id), state text NOT NULL DEFAULT 'pending',
 attempts integer NOT NULL DEFAULT 0, lifetime_attempts INTEGER NOT NULL DEFAULT 0,
 due_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), owner BLOB, generation INTEGER,
 started INTEGER NOT NULL DEFAULT false, outcome TEXT, checked_at TEXT,
 first_seen TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), last_seen TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 discovered_run BLOB REFERENCES runs(id), parent text
);
CREATE UNIQUE INDEX product_identity ON jobs(target,identity) WHERE kind='product';
CREATE UNIQUE INDEX sitemap_identity ON jobs(run,target,identity) WHERE kind='sitemap';
CREATE INDEX jobs_eligible ON jobs(run,state,due_at,id);
CREATE INDEX jobs_totals ON jobs(kind,state);
CREATE TABLE discoveries (
 job INTEGER NOT NULL REFERENCES jobs(id), source text NOT NULL, alias text NOT NULL,
 first_seen TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), last_seen TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 lastmod text, run BLOB NOT NULL REFERENCES runs(id), PRIMARY KEY(job,source,alias)
);
CREATE INDEX discoveries_run ON discoveries(run,job);
CREATE TABLE intents (
 id BLOB PRIMARY KEY, target text NOT NULL, product_id text NOT NULL,
 payload TEXT NOT NULL, generation INTEGER NOT NULL, finalized INTEGER NOT NULL DEFAULT false,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX pending_intents ON intents(id) WHERE NOT finalized;
CREATE TABLE products (
 target text NOT NULL, product_id text NOT NULL, title text NOT NULL CHECK(length(trim(title))>0),
 description_text text NOT NULL CHECK(length(trim(description_text))>0), publisher text,
 price TEXT, currency text, page_count INTEGER, language text,
 record TEXT NOT NULL, parser_version text NOT NULL, record_schema_version integer NOT NULL,
 archive TEXT NOT NULL, fetched_at TEXT NOT NULL, last_checked_at TEXT NOT NULL,
 saved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), reparsed_at TEXT,
 intent BLOB NOT NULL REFERENCES intents(id), PRIMARY KEY(target,product_id)
);
CREATE INDEX product_publisher ON products(publisher);
CREATE INDEX product_price ON products(currency,price);
CREATE INDEX product_parser ON products(parser_version,target,product_id);
CREATE TABLE attempts (
 id INTEGER PRIMARY KEY AUTOINCREMENT, job INTEGER REFERENCES jobs(id), run BLOB NOT NULL REFERENCES runs(id),
 stage text NOT NULL, outcome TEXT NOT NULL, at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 parser_version text NOT NULL
);
CREATE INDEX attempts_run ON attempts(run,id);
CREATE TABLE reprocess_outcomes (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 run BLOB NOT NULL REFERENCES runs(id), target text NOT NULL, product_id text NOT NULL,
 outcome TEXT NOT NULL, at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), UNIQUE(run,target,product_id)
);
CREATE VIEW catalogue_records AS
 SELECT p.*, json_object('first_seen',j.first_seen,'last_seen',j.last_seen,
 'sightings',json(COALESCE((SELECT json_group_array(json_object('source',d.source,'alias',d.alias,'first_seen',d.first_seen,'last_seen',d.last_seen,'lastmod',d.lastmod)) FROM discoveries d WHERE d.job=j.id),'[]'))) AS discovery,
 json_object('state',j.state,'attempts',j.lifetime_attempts,'checked_at',j.checked_at,'outcome',json(j.outcome)) AS crawl_status
 FROM products p JOIN jobs j ON j.target=p.target AND j.identity=p.product_id AND j.kind='product';
