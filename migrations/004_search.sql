-- Derived search projection. Canonical products, queues and archives stay untouched.
CREATE TABLE IF NOT EXISTS search_documents (
 id INTEGER PRIMARY KEY, target TEXT NOT NULL, product_id TEXT NOT NULL,
 title TEXT NOT NULL COLLATE NOCASE, description_text TEXT NOT NULL, publisher TEXT,
 price TEXT COLLATE exact_decimal, currency TEXT, page_count INTEGER,
 rating REAL, rating_count INTEGER, review_count INTEGER,
 available_on TEXT, captured_on TEXT NOT NULL, authors TEXT NOT NULL,
 facets TEXT NOT NULL, flags TEXT NOT NULL, identifiers TEXT NOT NULL,
 creators TEXT NOT NULL, taxonomy TEXT NOT NULL,
 UNIQUE(target,product_id)
);
CREATE INDEX IF NOT EXISTS search_title ON search_documents(title,id);
CREATE INDEX IF NOT EXISTS search_captured ON search_documents(captured_on,id);
CREATE INDEX IF NOT EXISTS search_price ON search_documents(currency,price,id);
CREATE INDEX IF NOT EXISTS search_pages ON search_documents(page_count,id);
CREATE INDEX IF NOT EXISTS search_rating ON search_documents(rating,id);
CREATE INDEX IF NOT EXISTS search_available ON search_documents(available_on,id);
CREATE TABLE IF NOT EXISTS search_facets (
 document INTEGER NOT NULL REFERENCES search_documents(id) ON DELETE CASCADE,
 field TEXT NOT NULL, value TEXT NOT NULL,
 PRIMARY KEY(document,field,value)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS search_facet_values ON search_facets(field,value,document);
CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
 title, description_text, publisher, identifiers, creators, taxonomy,
 content='search_documents', content_rowid='id', tokenize='unicode61 remove_diacritics 2', prefix='2 3'
);
CREATE TABLE IF NOT EXISTS search_progress(singleton INTEGER PRIMARY KEY CHECK(singleton=1), last_id INTEGER NOT NULL, ready INTEGER NOT NULL);
INSERT INTO search_progress VALUES(1,0,0) ON CONFLICT DO NOTHING;
CREATE VIEW IF NOT EXISTS search_source AS SELECT
 p.rowid AS id,p.target,p.product_id,p.title,p.description_text,p.publisher,p.price,p.currency,p.page_count,
 CASE WHEN json_type(p.record,'$.rating') IN ('integer','real','text') THEN CAST(json_extract(p.record,'$.rating') AS REAL) END AS rating,
 json_extract(p.record,'$.rating_count') AS rating_count,json_extract(p.record,'$.review_count') AS review_count,
 CASE WHEN json_extract(p.record,'$.date_available') GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*' THEN date(json_extract(p.record,'$.date_available')) END AS available_on,
 p.fetched_at AS captured_on,coalesce(json_extract(p.record,'$.authors'),'[]') AS authors,
 json_object(
 'publisher',json(json_array(p.publisher)),
 'authors',json(coalesce(json_extract(p.record,'$.authors'),'[]')),
 'artists',json(coalesce(json_extract(p.record,'$.artists'),'[]')),
 'editors',json(coalesce(json_extract(p.record,'$.editors'),'[]')),
 'contributors',json(coalesce(json_extract(p.record,'$.contributors'),'[]')),
 'genres',json(coalesce(json_extract(p.record,'$.genres'),'[]')),
 'themes',json(coalesce(json_extract(p.record,'$.themes'),'[]')),
 'rule_systems',json(coalesce(json_extract(p.record,'$.rule_systems'),'[]')),
 'product_types',json(coalesce(json_extract(p.record,'$.product_types'),'[]')),
 'languages',json(coalesce(json_extract(p.record,'$.languages'),'[]')),
 'formats',json(coalesce(json_extract(p.record,'$.formats'),'[]')),
 'categories',json(coalesce((SELECT json_group_array(json_extract(value,'$.name')) FROM json_each(p.record,'$.categories')),'[]')),
 'tags',json(coalesce((SELECT json_group_array(json_extract(value,'$.name')) FROM json_each(p.record,'$.filters')),'[]')),
 'primary_tags',json(coalesce((SELECT json_group_array(json_extract(value,'$.name')) FROM json_each(p.record,'$.primary_filters')),'[]')),
 'currency',json(json_array(p.currency)),
 'edition',json(json_array(json_extract(p.record,'$.edition'))),
 'content_rating',json(json_array(json_extract(p.record,'$.flags.contentRating'))),
 'status',json(json_array(json_extract(p.record,'$.flags.status')))) AS facets,
 json_object(
'pwyw',CASE lower(CAST(json_extract(p.record,'$.flags.isPwyw') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'community',CASE lower(CAST(json_extract(p.record,'$.flags.isCommunityContent') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'audio',CASE lower(CAST(json_extract(p.record,'$.flags.isAudio') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'video',CASE lower(CAST(json_extract(p.record,'$.flags.isVideo') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'scanned',CASE lower(CAST(json_extract(p.record,'$.flags.scannedPDF') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'watermarked',CASE lower(CAST(json_extract(p.record,'$.flags.watermarked') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'enabled',CASE lower(CAST(json_extract(p.record,'$.flags.enabled') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'bundle',CASE lower(CAST(json_extract(p.record,'$.is_bundle') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'on_sale',CASE lower(CAST(json_extract(p.record,'$.on_sale') AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'ai',CASE lower(CAST(coalesce(json_extract(p.record,'$.flags.isAi'),json_extract(p.record,'$.flags.ai')) AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END,
'handmade',CASE lower(CAST(coalesce(json_extract(p.record,'$.flags.isHandmade'),json_extract(p.record,'$.flags.handmade')) AS TEXT)) WHEN 'true' THEN 1 WHEN '1' THEN 1 WHEN 'false' THEN 0 WHEN '0' THEN 0 END) AS flags,
 p.product_id||' '||coalesce(json_extract(p.record,'$.isbn'),'')||' '||coalesce(json_extract(p.record,'$.sku'),'') AS identifiers,
 coalesce(json_extract(p.record,'$.authors'),'')||' '||coalesce(json_extract(p.record,'$.artists'),'')||' '||coalesce(json_extract(p.record,'$.editors'),'')||' '||coalesce(json_extract(p.record,'$.contributors'),'') AS creators,
 coalesce(json_extract(p.record,'$.genres'),'')||' '||coalesce(json_extract(p.record,'$.themes'),'')||' '||coalesce(json_extract(p.record,'$.rule_systems'),'')||' '||coalesce(json_extract(p.record,'$.product_types'),'')||' '||coalesce(json_extract(p.record,'$.languages'),'')||' '||coalesce(json_extract(p.record,'$.formats'),'')||' '||coalesce((SELECT group_concat(json_extract(value,'$.name'),' ') FROM json_each(p.record,'$.categories')),'')||' '||coalesce((SELECT group_concat(json_extract(value,'$.name'),' ') FROM json_each(p.record,'$.filters')),'')||' '||coalesce((SELECT group_concat(json_extract(value,'$.name'),' ') FROM json_each(p.record,'$.primary_filters')),'') AS taxonomy
 FROM products p;
CREATE TRIGGER IF NOT EXISTS search_document_insert AFTER INSERT ON search_documents BEGIN
 INSERT INTO search_fts(rowid,title,description_text,publisher,identifiers,creators,taxonomy) VALUES(new.id,new.title,new.description_text,new.publisher,new.identifiers,new.creators,new.taxonomy);
 INSERT OR IGNORE INTO search_facets(document,field,value)
 SELECT new.id,f.key,trim(v.value) FROM json_each(new.facets) f,json_each(f.value) v
 WHERE v.type='text' AND length(trim(v.value))>0;
END;
CREATE TRIGGER IF NOT EXISTS search_document_delete AFTER DELETE ON search_documents BEGIN
 INSERT INTO search_fts(search_fts,rowid,title,description_text,publisher,identifiers,creators,taxonomy) VALUES('delete',old.id,old.title,old.description_text,old.publisher,old.identifiers,old.creators,old.taxonomy);
END;
CREATE TRIGGER IF NOT EXISTS search_product_insert AFTER INSERT ON products BEGIN
 INSERT INTO search_documents SELECT * FROM search_source WHERE id=new.rowid;
END;
CREATE TRIGGER IF NOT EXISTS search_product_update AFTER UPDATE OF title,description_text,publisher,price,currency,page_count,record,fetched_at ON products BEGIN
 DELETE FROM search_documents WHERE id=old.rowid;
 INSERT INTO search_documents SELECT * FROM search_source WHERE id=new.rowid;
END;
CREATE TRIGGER IF NOT EXISTS search_product_delete AFTER DELETE ON products BEGIN
 DELETE FROM search_documents WHERE id=old.rowid;
END;
