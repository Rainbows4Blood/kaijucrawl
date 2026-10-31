//! Pure, bounded API extraction. Raw values retain JSON's original decimal precision.
use crate::{domain::*, targets::drivethru as d};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use url::Url;

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().into()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
fn first<'a>(vs: &[&'a Value]) -> &'a Value {
    vs.iter()
        .copied()
        .find(|v| !v.is_null() && v.as_str() != Some(""))
        .unwrap_or(&Value::Null)
}
fn list(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().collect(),
        Value::Null => vec![],
        _ => vec![v],
    }
}
fn unique(xs: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    xs.into_iter()
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}
pub fn decimal(v: &Value) -> Value {
    let Some(s) = scalar(v) else {
        return Value::Null;
    };
    // JSON number grammar rejects booleans, NaN and infinities. Return decimal text,
    // not f64: both very small and high precision source amounts remain exact.
    match serde_json::from_str::<Value>(&s) {
        Ok(Value::Number(_)) => Value::String(s),
        _ => Value::Null,
    }
}
fn integer(v: &Value) -> Value {
    let Some(s) = scalar(v) else {
        return Value::Null;
    };
    if let Ok(n) = s.parse::<i64>() {
        return json!(n);
    }
    if let Some((a, b)) = s.split_once('.')
        && !b.is_empty()
        && b.bytes().all(|c| c == b'0')
    {
        return a.parse::<i64>().map_or(Value::Null, |n| json!(n));
    }
    Value::Null
}
fn id(v: &Value) -> Option<String> {
    let s = scalar(v)?;
    ProductId::new(&s)
        .ok()
        .or_else(|| d::product_url(&s))
        .map(|x| x.as_str().into())
}
fn ids(v: &Value) -> Vec<String> {
    unique(
        ["productId", "product_id", "productID", "id", "@id", "url"]
            .iter()
            .filter_map(|k| id(&v[k])),
    )
}
fn schema(reason: &str) -> Error {
    Failure::new(FailureKind::Schema, reason).into()
}

pub fn clean(s: &str) -> (String, String) {
    let html = ammonia::Builder::default()
        .add_clean_content_tags(&[
            "script", "style", "noscript", "template", "iframe", "object", "embed",
        ])
        .clean(s)
        .to_string();
    let doc = scraper::Html::parse_fragment(&html);
    let mut text = String::new();
    // Edge traversal is iterative and visits each node twice; inline tags add no spaces.
    for edge in doc.tree.root().traverse() {
        match edge {
            ego_tree::iter::Edge::Open(node) => match node.value() {
                scraper::node::Node::Text(t) => text.push_str(t),
                scraper::node::Node::Element(e) => match e.name() {
                    "li" => text.push_str("\n• "),
                    "br" => text.push('\n'),
                    "p" | "div" | "section" | "article" | "blockquote" | "h1" | "h2" | "h3"
                    | "h4" | "h5" | "h6" | "ul" | "ol" | "table" => text.push_str("\n\n"),
                    _ => {}
                },
                _ => {}
            },
            ego_tree::iter::Edge::Close(node) => {
                if let scraper::node::Node::Element(e) = node.value() {
                    match e.name() {
                        "td" | "th" => text.push('\t'),
                        "li" | "tr" => text.push('\n'),
                        "p" | "div" | "section" | "article" | "blockquote" | "table" => {
                            text.push_str("\n\n")
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let mut lines = Vec::new();
    let mut empty = false;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() || !empty {
            lines.push(line.clone());
        }
        empty = line.is_empty();
    }
    (html.trim().into(), lines.join("\n").trim().into())
}
fn plain(v: &Value) -> Value {
    scalar(v)
        .map(|s| Value::String(clean(&s).1))
        .filter(|v| v.as_str() != Some(""))
        .unwrap_or(Value::Null)
}
fn names(v: &Value) -> Vec<String> {
    unique(list(v).iter().filter_map(|x| {
        plain(if x.is_object() {
            first(&[
                &x["name"],
                &x["displayName"],
                &x["fullName"],
                &x["languageName"],
            ])
        } else {
            x
        })
        .as_str()
        .map(str::to_owned)
    }))
}
fn resolve(v: &Value, base: &str) -> Value {
    scalar(v)
        .and_then(|s| Url::parse(base).ok()?.join(&s).ok())
        .filter(|u| {
            ["http", "https"].contains(&u.scheme())
                && u.username().is_empty()
                && u.password().is_none()
        })
        .map(|u| json!(u.as_str()))
        .unwrap_or(Value::Null)
}
fn gates(v: &Value) -> Result<()> {
    for k in [
        "ageGate",
        "ageGateRequired",
        "requiresAgeVerification",
        "adultContentRestricted",
    ] {
        if v[k] == true {
            return Err(Failure::new(
                FailureKind::Restricted,
                "Explicit individual content restriction",
            )
            .into());
        }
    }
    for k in ["loginRequired", "authenticationRequired", "accessDenied"] {
        if v[k] == true {
            return Err(Failure::new(
                FailureKind::Authentication,
                "Explicit authentication denial",
            )
            .into());
        }
    }
    let message = ["error", "errors", "message", "detail"]
        .iter()
        .filter_map(|k| scalar(&v[k]))
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(f) = d::classify(200, message.as_bytes()) {
        return Err(f.into());
    }
    if !v["error"].is_null() || !v["errors"].is_null() {
        return Err(schema("API error envelope"));
    }
    Ok(())
}
fn unwrap(payload: &Value, expected: &str) -> Result<(Value, Value)> {
    gates(payload)?;
    let root = if payload.is_object() && ids(payload).is_empty() {
        first(&[&payload["data"], &payload["product"], payload])
    } else {
        payload
    };
    let mut matched = Vec::new();
    for item in list(root) {
        if !item.is_object() {
            continue;
        }
        let p = if item["attributes"].is_object() {
            &item["attributes"]
        } else {
            item
        };
        let found = unique(ids(item).into_iter().chain(ids(p)));
        if found.iter().any(|i| i == expected) {
            if found.iter().any(|i| i != expected) {
                return Err(schema("Conflicting product identities"));
            }
            matched.push((item, p));
        }
    }
    if matched.len() != 1 {
        return Err(schema("No unique matching direct product"));
    }
    let (item, accepted) = matched[0];
    gates(accepted)?;
    if ![
        "description",
        "name",
        "title",
        "publisher",
        "options",
        "price",
        "authors",
    ]
    .iter()
    .any(|k| accepted.get(*k).is_some())
    {
        return Err(schema("ID-only product envelope"));
    }
    let mut resolved = accepted.clone();
    if let Some(rel) = item["relationships"].as_object() {
        for (name, r) in rel {
            if !["publisher", "authors", "artists", "editors", "contributors"]
                .contains(&name.as_str())
                || resolved.get(name).is_some()
            {
                continue;
            }
            let mut values = Vec::new();
            for reference in list(&r["data"]) {
                let candidates: Vec<_> = list(&payload["included"])
                    .into_iter()
                    .filter(|x| {
                        x["id"] == reference["id"]
                            && (reference["type"].is_null() || x["type"] == reference["type"])
                    })
                    .collect();
                if candidates.len() == 1 {
                    let x = candidates[0];
                    let mut v = if x["attributes"].is_object() {
                        x["attributes"].clone()
                    } else {
                        x.clone()
                    };
                    if v.get("id").is_none() {
                        v["id"] = x["id"].clone();
                    }
                    values.push(v);
                }
            }
            resolved[name] = if r["data"].is_array() {
                json!(values)
            } else {
                values.into_iter().next().unwrap_or(Value::Null)
            };
        }
    }
    Ok((accepted.clone(), resolved))
}
fn taxonomy(v: &Value, kind: &str) -> Vec<Value> {
    list(v).iter().map(|x|{
    if !x.is_object(){return json!({"name":plain(x),"kind":kind});}
    let descriptions=list(&x["descriptions"]);let preferred=descriptions.iter().find(|v|v["languageCode"]=="en"||v["languageId"]==1).copied().or_else(||descriptions.first().copied()).unwrap_or(&Value::Null);
    let ancestors:Vec<_>=list(&x["ancestors"]).iter().map(|v|integer(v)).filter(|v|!v.is_null()).collect();let parent=integer(&x["parentId"]);
    let group=if kind=="filter"{ancestors.iter().chain(std::iter::once(&parent)).find_map(|v|match v.as_i64(){Some(10)=>Some("genre"),Some(20)=>Some("product_type"),Some(30)=>Some("rule_system"),Some(40)=>Some("language"),Some(44498)=>Some("format"),_=>None})}else{None};
    json!({"id":scalar(first(&[&x["filterId"],&x["categoryId"],&x["id"]])),"name":plain(first(&[&x["name"],&preferred["name"]])),"kind":kind,"group":group,"parent_id":parent,"ancestor_ids":ancestors,"slug":scalar(first(&[&x["slug"],&preferred["slug"]])),"localized_labels":x.get("descriptions").cloned().unwrap_or(json!([])),"raw":x})
}).collect()
}

pub fn parse(body: &[u8], expected: &str, api_url: &str) -> Result<Product> {
    if body.len() > PRODUCT_LIMIT {
        return Err(schema("Product exceeds decoded body limit"));
    }
    let expected = ProductId::new(expected)?;
    let expected_url =
        Url::parse(&d::detail(expected.as_str())).map_err(|_| schema("Invalid API URL"))?;
    let actual = Url::parse(api_url).map_err(|_| schema("Invalid API URL"))?;
    if !d::redirect(&expected_url, &actual, Some(expected.as_str())) {
        return Err(schema("API URL identity or scope mismatch"));
    }
    let payload: Value = serde_json::from_slice(body).map_err(|_| schema("Malformed JSON"))?;
    let (accepted, p) = unwrap(&payload, expected.as_str())?;
    let desc = &p["description"];
    let title = plain(first(&[&desc["name"], &p["name"], &p["title"]]))
        .as_str()
        .unwrap_or("")
        .to_owned();
    let full = first(&[
        &desc["description"],
        if desc.is_string() { desc } else { &Value::Null },
        &p["fullDescription"],
    ]);
    let (html, text) = clean(full.as_str().unwrap_or(""));
    let mut r = Map::new();
    r.insert("description_html".into(), json!(html));
    r.insert(
        "short_description".into(),
        plain(first(&[&desc["shortDescription"], &p["shortDescription"]])),
    );
    r.insert("source_kind".into(), json!("public_api"));
    r.insert("source_url".into(), json!(api_url));
    r.insert("api_url".into(), json!(api_url));
    r.insert("url".into(), json!(d::canonical(expected.as_str())));
    let slug = scalar(first(&[&desc["slug"], &p["slug"]]));
    let mut product_url = Url::parse(&d::canonical(expected.as_str()))
        .map_err(|_| schema("Invalid canonical URL"))?;
    if let Some(s) = &slug {
        product_url
            .path_segments_mut()
            .map_err(|_| schema("Invalid product URL"))?
            .push(s);
    }
    r.insert("slug".into(), json!(slug));
    r.insert("product_url".into(), json!(product_url.as_str()));
    let publisher = &p["publisher"];
    r.insert(
        "publisher".into(),
        plain(if publisher.is_object() {
            &publisher["name"]
        } else {
            publisher
        }),
    );
    r.insert(
        "publisher_id".into(),
        json!(scalar(first(&[
            &p["publisherId"],
            &publisher["publisherId"],
            &publisher["id"]
        ]))),
    );
    r.insert("publisher_metadata".into(), publisher.clone());
    let currency = first(&[&p["currency"], &p["priceCurrency"], &p["currencyCode"]]);
    let currency = scalar(if currency.is_object() {
        first(&[&currency["code"], &currency["currencyCode"]])
    } else {
        currency
    });
    r.insert(
        "currency_source".into(),
        json!(if currency.is_some() {
            "api_response"
        } else {
            "frontend_base_currency"
        }),
    );
    let currency = currency.unwrap_or(d::FRONTEND_CURRENCY.into());
    r.insert("currency".into(), json!(currency));
    let base = decimal(&p["price"]);
    let special = decimal(&p["specialPrice"]);
    r.insert(
        "price".into(),
        if p["onSale"] == true && !special.is_null() {
            special.clone()
        } else {
            base.clone()
        },
    );
    r.insert("base_price".into(), base);
    r.insert("special_price".into(), special);
    r.insert("on_sale".into(), p["onSale"].clone());
    r.insert("lowest_prices".into(),json!({"digital":decimal(&p["lowestDigitalPrice"]),"print":decimal(&p["lowestPrintPrice"]),"any":decimal(&p["lowestAnyPrice"])}));
    let offers:Vec<_>=list(&p["options"]).iter().filter(|v|v.is_object()).map(|v|json!({"id":scalar(&v["productAttributeId"]),"format":plain(first(&[&v["optionValueName"],&v["description"]])),"option_name":plain(&v["optionName"]),"type":v["type"],"price":decimal(&v["price"]),"currency":first(&[&v["currency"],&r["currency"]]),"price_to_add":decimal(&v["priceToAdd"]),"msrp":decimal(&v["msrp"]),"enabled":v["enabled"],"raw":v})).collect();
    r.insert(
        "formats".into(),
        json!(unique(offers.iter().filter_map(|v| scalar(&v["format"])))),
    );
    r.insert("offers".into(), json!(offers));
    let mut credits = vec![];
    for (key, role) in [
        ("authors", "author"),
        ("artists", "artist"),
        ("editors", "editor"),
        ("contributors", "contributor"),
    ] {
        let ns = names(&p[key]);
        for n in &ns {
            credits.push(json!({"role":role,"name":n}));
        }
        r.insert(key.into(), json!(ns));
    }
    r.insert("credits".into(), json!(credits));
    r.insert(
        "page_count".into(),
        integer(first(&[
            &p["pagecount"],
            &p["pageCount"],
            &p["numberOfPages"],
        ])),
    );
    for k in ["isbn", "sku", "edition"] {
        r.insert(k.into(), json!(scalar(&p[k])));
    }
    let dates: Map<_, _> = p
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| {
            k.starts_with("date") || ["fileLastModified", "publicationDate"].contains(&k.as_str())
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    r.insert("dates".into(), json!(dates));
    for (to, from) in [
        ("date_created", "dateCreated"),
        ("date_modified", "dateModified"),
        ("date_available", "dateAvailable"),
        ("file_last_modified", "fileLastModified"),
    ] {
        r.insert(to.into(), p[from].clone());
    }
    let filters = taxonomy(&p["filters"], "filter");
    let primary = taxonomy(&p["storefrontPrimaryFilterValues"], "filter");
    let mut groups = Map::new();
    for group in ["genre", "product_type", "rule_system", "language", "format"] {
        groups.insert(
            group.into(),
            json!(unique(
                filters
                    .iter()
                    .chain(primary.iter())
                    .filter(|v| v["group"] == group)
                    .filter_map(|v| scalar(&v["name"]))
            )),
        );
    }
    for (key, singular, group) in [
        ("genres", "genre", "genre"),
        ("themes", "theme", "theme"),
        ("rule_systems", "ruleSystem", "rule_system"),
        ("product_types", "productType", "product_type"),
        ("languages", "language", "language"),
    ] {
        let plural = match key {
            "rule_systems" => "ruleSystems",
            "product_types" => "productTypes",
            _ => key,
        };
        let mut ns = groups.get(group).map(names).unwrap_or_default();
        ns.extend(names(first(&[
            &p[plural],
            &p[singular],
            if key == "languages" {
                &p["inLanguage"]
            } else {
                &Value::Null
            },
        ])));
        r.insert(key.into(), json!(unique(ns)));
    }
    r.insert("language".into(), {
        let ns = names(&r["languages"]);
        if ns.is_empty() {
            Value::Null
        } else {
            json!(ns.join(", "))
        }
    });
    r.insert("filter_groups".into(), json!(groups));
    r.insert("filters".into(), json!(filters));
    r.insert("primary_filters".into(), json!(primary));
    r.insert(
        "categories".into(),
        json!(taxonomy(&p["categories"], "category")),
    );
    let ratings = &p["reviewRatings"];
    let mut count = integer(&p["ratingCount"]);
    if count.is_null() {
        let buckets: Option<Vec<_>> = [
            "countOne",
            "countTwo",
            "countThree",
            "countFour",
            "countFive",
        ]
        .iter()
        .map(|k| integer(&ratings[k]).as_i64().filter(|n| *n >= 0))
        .collect();
        if let Some(xs) = buckets {
            count = xs
                .into_iter()
                .try_fold(0i64, |a, b| a.checked_add(b))
                .map_or(Value::Null, |n| json!(n));
        }
    }
    let mut rating = decimal(first(&[&p["rating"], &ratings["average"]]));
    if count == 0
        && rating
            .as_str()
            .is_some_and(|s| s.chars().all(|c| c == '0' || c == '.'))
    {
        rating = Value::Null;
    }
    r.insert("rating".into(), rating);
    r.insert("rating_count".into(), count);
    r.insert("review_count".into(), integer(&p["reviewCount"]));
    r.insert("review_ratings".into(), ratings.clone());
    r.insert(
        "reviews".into(),
        p.get("reviews").cloned().unwrap_or(json!([])),
    );
    r.insert("ranking".into(), p["ranking"].clone());
    r.insert("files".into(),json!(list(&p["files"]).iter().filter(|v|v.is_object()).map(|v|json!({"filename":scalar(first(&[&v["filename"],&v["name"]])),"title":plain(&v["title"]),"size_bytes":integer(&v["size"]),"size_mb_reported":v["sizeMB"],"raw":v})).collect::<Vec<_>>()));
    r.insert("file_size_reported".into(), p["filesize"].clone());
    let mut images = vec![];
    for key in [
        "webImage",
        "image",
        "thumbnail",
        "thumbnail200",
        "thumbnail100",
        "thumbnail80",
        "thumbnail40",
    ] {
        let u = resolve(&p[key], d::IMAGE);
        if !u.is_null() {
            images.push(json!({"kind":key,"url":u}));
        }
    }
    for v in list(&p["associatedImages"]) {
        let u = resolve(
            if v.is_object() {
                first(&[&v["url"], &v["image"], &v["src"], &v["filename"]])
            } else {
                v
            },
            d::IMAGE,
        );
        if !u.is_null() {
            images.push(json!({"kind":"associated","url":u,"alt":scalar(&v["alt"]),"raw":v}));
        }
    }
    let mut links = vec![];
    let doc = scraper::Html::parse_fragment(&html);
    for node in doc.tree.nodes() {
        if let Some(e) = scraper::ElementRef::wrap(node) {
            if e.value().name() == "img" {
                let u = resolve(&json!(e.value().attr("src")), d::WEBSITE);
                if !u.is_null() {
                    images.push(json!({"kind":"description","url":u,"alt":e.value().attr("alt")}));
                }
            } else if e.value().name() == "a" {
                let u = resolve(&json!(e.value().attr("href")), d::WEBSITE);
                if !u.is_null() {
                    links.push(json!({"url":u,"text":e.text().collect::<String>()}));
                }
            }
        }
    }
    r.insert(
        "cover_image".into(),
        resolve(first(&[&p["webImage"], &p["image"]]), d::IMAGE),
    );
    r.insert("images".into(), json!(images));
    r.insert("description_links".into(), json!(links));
    let mut previews = vec![];
    for key in [
        "previewQuickFile",
        "previewFullFile",
        "previewCustomFile",
        "publisherPreviewFile",
        "previewAudioFile",
        "videoPreviewUrl",
        "previewVideoUrl",
    ] {
        let u = resolve(
            first(&[&desc[key], &p[key]]),
            if key == "previewFullFile" {
                d::PREVIEW
            } else {
                d::WEBSITE
            },
        );
        if !u.is_null() {
            previews.push(json!({"kind":key,"url":u}));
        }
    }
    r.insert("previews".into(), json!(previews));
    let mut related = vec![];
    for key in [
        "bundles",
        "bundleProducts",
        "productsInBundle",
        "bundledProducts",
        "bundleItems",
        "relatedProducts",
        "alsoPurchased",
        "recommendations",
        "related",
    ] {
        for v in list(&p[key]) {
            if let Some(pid) = if v.is_object() {
                ids(v).into_iter().next()
            } else {
                id(v)
            } {
                related.push(json!({"product_id":pid,"title":plain(first(&[&v["name"],&v["title"],&v["description"]["name"]])),"relation":key,"url":if v["url"].is_null(){json!(d::canonical(&pid))}else{resolve(&v["url"],d::WEBSITE)},"raw":v}));
            }
        }
    }
    r.insert("related_products".into(), json!(related));
    r.insert("is_bundle".into(), p["isBundle"].clone());
    r.insert("bundle_id".into(), json!(scalar(&p["bundleId"])));
    let flags: Map<_, _> = [
        "status",
        "enabled",
        "workflowStatus",
        "isAudio",
        "isVideo",
        "isPwyw",
        "isCommunityContent",
        "isBundle",
        "scannedPDF",
        "watermarked",
        "disableCp",
        "disablePrint",
        "giftCertificate",
        "ai",
        "handmade",
        "isGiftCert",
        "isAi",
        "isHandmade",
        "contentRating",
    ]
    .iter()
    .filter_map(|k| p.get(*k).map(|v| (k.to_string(), v.clone())))
    .collect();
    r.insert("flags".into(), json!(flags));
    r.insert(
        "raw_metadata".into(),
        if accepted == payload {
            json!({"public_api":accepted})
        } else {
            json!({"public_api":accepted,"api_response":payload})
        },
    );
    r.insert("parser_version".into(), json!(PARSER_VERSION));
    r.insert("record_schema_version".into(), json!(SCHEMA_VERSION));
    r.insert("extraction_status".into(), json!("complete"));
    r.insert("missing_fields".into(), json!([]));
    let product = Product {
        target_key: d::KEY.into(),
        product_id: expected,
        title,
        description_text: text,
        fields: r,
    };
    product.validate()?;
    Ok(product)
}
