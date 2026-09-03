//! vim25 SOAP client.
//!
//! Everything that REST does not expose comes from here: `RetrievePropertiesEx`
//! on the property collector, over a `ContainerView` of one managed-object type.

use super::config::VCenterConnection;
use super::xml::{self, Element};

pub const SOAP_ACTION: &str = "urn:vim25/8.0";

/// Escape text destined for a SOAP envelope.
///
/// Passwords are interpolated into XML; one containing `&` or `<` produces a
/// malformed envelope and a failure that looks like bad credentials.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn envelope(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:vim25="urn:vim25"><soapenv:Body>{body}</soapenv:Body></soapenv:Envelope>"#
    )
}

pub struct SoapClient {
    http: reqwest::Client,
    url: String,
    /// `vmware_soap_session` cookie value, set by `login`.
    pub cookie: Option<String>,
}

impl SoapClient {
    pub fn new(conn: &VCenterConnection) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(conn.skip_cert_verify)
            .build()
            .map_err(|e| format!("could not build HTTP client: {e}"))?;
        Ok(Self {
            http,
            url: format!("{}/sdk", conn.base_url()),
            cookie: None,
        })
    }

    /// Send one SOAP call and return the parsed `<Body>` contents.
    pub async fn call(&self, body: &str) -> Result<Element, String> {
        let mut req = self
            .http
            .post(&self.url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("SOAPAction", SOAP_ACTION)
            .body(envelope(body));
        if let Some(c) = &self.cookie {
            req = req.header("Cookie", c.clone());
        }

        let resp = req.send().await.map_err(|e| format!("SOAP request failed: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("could not read SOAP response: {e}"))?;
        let root = xml::parse(&text)?;

        if let Some(fault) = root.find("Fault") {
            let msg = fault
                .find("localizedMessage")
                .map(|e| e.text.clone())
                .or_else(|| fault.find("faultstring").map(|e| e.text.clone()))
                .unwrap_or_else(|| "unspecified SOAP fault".into());
            return Err(format!("vCenter SOAP fault ({status}): {msg}"));
        }
        if !status.is_success() {
            return Err(format!("SOAP call returned HTTP {status}"));
        }

        root.find("Body")
            .cloned()
            .ok_or_else(|| "SOAP response had no Body".to_string())
    }

    pub async fn login(&mut self, conn: &VCenterConnection) -> Result<(), String> {
        let body = format!(
            r#"<vim25:Login><vim25:_this type="SessionManager">SessionManager</vim25:_this><vim25:userName>{}</vim25:userName><vim25:password>{}</vim25:password></vim25:Login>"#,
            xml_escape(&conn.username),
            xml_escape(&conn.password)
        );

        // The session cookie only arrives on this response, so send it by hand
        // rather than through `call`, which needs the cookie to already exist.
        let resp = self
            .http
            .post(&self.url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("SOAPAction", SOAP_ACTION)
            .body(envelope(&body))
            .send()
            .await
            .map_err(|e| format!("SOAP login failed: {e}"))?;

        let cookie = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| v.split(';').next().map(str::to_string))
            .filter(|v| v.starts_with("vmware_soap_session"));

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("could not read login response: {e}"))?;
        let root = xml::parse(&text)?;
        if let Some(fault) = root.find("Fault") {
            let msg = fault
                .find("localizedMessage")
                .map(|e| e.text.clone())
                .unwrap_or_else(|| "login rejected".into());
            return Err(format!("SOAP login failed: {msg}"));
        }
        if !status.is_success() {
            return Err(format!("SOAP login returned HTTP {status}"));
        }

        self.cookie = Some(cookie.ok_or("SOAP login returned no session cookie")?);
        Ok(())
    }

    pub async fn logout(&self) -> Result<(), String> {
        if self.cookie.is_none() {
            return Ok(());
        }
        self.call(r#"<vim25:Logout><vim25:_this type="SessionManager">SessionManager</vim25:_this></vim25:Logout>"#)
            .await
            .map(|_| ())
    }

    /// `<type>` repeats, so one view can span several managed-object types.
    /// Verified against the lab: a view over Folder + Datacenter +
    /// ComputeResource returns all three, and a `ComputeResource` view also
    /// yields `ClusterComputeResource`, which is its subclass.
    async fn create_container_view(&self, obj_types: &[&str]) -> Result<String, String> {
        let types: String = obj_types
            .iter()
            .map(|t| format!("<vim25:type>{}</vim25:type>", xml_escape(t)))
            .collect();
        let body = format!(
            r#"<vim25:CreateContainerView><vim25:_this type="ViewManager">ViewManager</vim25:_this><vim25:container type="Folder">group-d1</vim25:container>{types}<vim25:recursive>true</vim25:recursive></vim25:CreateContainerView>"#
        );
        let body = self.call(&body).await?;
        body.find("returnval")
            .map(|e| e.text.clone())
            .ok_or_else(|| format!("CreateContainerView for {obj_types:?} returned no view"))
    }

    async fn destroy_view(&self, view: &str) -> Result<(), String> {
        let body = format!(
            r#"<vim25:DestroyView><vim25:_this type="ContainerView">{}</vim25:_this></vim25:DestroyView>"#,
            xml_escape(view)
        );
        self.call(&body).await.map(|_| ())
    }

    /// Retrieve `props` for every managed object of `obj_type` in the inventory.
    ///
    /// Follows the continuation token: a truncated result that is silently
    /// accepted would under-report the inventory, which is the worst failure
    /// mode this tool has.
    pub async fn retrieve(
        &self,
        obj_type: &str,
        props: &[&str],
    ) -> Result<Vec<ManagedObject>, String> {
        self.retrieve_types(&[obj_type], props).await
    }

    /// Retrieve the same `props` for every object of any type in `obj_types`.
    ///
    /// One view and one `RetrievePropertiesEx` covers the lot, which is what
    /// keeps the inventory path index down to a single extra walk instead of
    /// one per type. Each type needs its own `propSet` entry even though the
    /// property list is shared.
    pub async fn retrieve_types(
        &self,
        obj_types: &[&str],
        props: &[&str],
    ) -> Result<Vec<ManagedObject>, String> {
        let view = self.create_container_view(obj_types).await?;
        let result = self.retrieve_with_view(obj_types, props, &view).await;
        // Views accumulate on the session until logout; drop it either way.
        if let Err(e) = self.destroy_view(&view).await {
            eprintln!("warning: could not destroy {obj_types:?} container view: {e}");
        }
        result
    }

    /// Retrieve properties of **one** managed object, addressed by its own
    /// moref rather than through a ContainerView.
    ///
    /// Singletons like `LicenseManager` are not in any container, so the
    /// ContainerView shape every other fetch uses cannot reach them at all.
    /// Returns `None` when the object exists but reports none of the requested
    /// properties.
    pub async fn retrieve_moref(
        &self,
        obj_type: &str,
        moref: &str,
        props: &[&str],
    ) -> Result<Option<ManagedObject>, String> {
        let path_set: String = props
            .iter()
            .map(|p| format!("<vim25:pathSet>{}</vim25:pathSet>", xml_escape(p)))
            .collect();
        let body = format!(
            r#"<vim25:RetrievePropertiesEx><vim25:_this type="PropertyCollector">propertyCollector</vim25:_this><vim25:specSet><vim25:propSet><vim25:type>{obj_type}</vim25:type>{path_set}</vim25:propSet><vim25:objectSet><vim25:obj type="{obj_type}">{}</vim25:obj></vim25:objectSet></vim25:specSet><vim25:options/></vim25:RetrievePropertiesEx>"#,
            xml_escape(moref)
        );
        let page = self.call(&body).await?;
        Ok(page
            .find("returnval")
            .and_then(|r| r.children_named("objects").next().map(ManagedObject::from_element)))
    }

    /// `ServiceContent`, which needs no ContainerView and names the singleton
    /// managers (`LicenseManager` among them). `about` on it is what vSource
    /// reports.
    pub async fn service_content(&self) -> Result<Element, String> {
        let body = self
            .call(
                r#"<vim25:RetrieveServiceContent><vim25:_this type="ServiceInstance">ServiceInstance</vim25:_this></vim25:RetrieveServiceContent>"#,
            )
            .await?;
        body.find("returnval")
            .cloned()
            .ok_or_else(|| "RetrieveServiceContent returned no content".to_string())
    }

    async fn retrieve_with_view(
        &self,
        obj_types: &[&str],
        props: &[&str],
        view: &str,
    ) -> Result<Vec<ManagedObject>, String> {
        let path_set: String = props
            .iter()
            .map(|p| format!("<vim25:pathSet>{}</vim25:pathSet>", xml_escape(p)))
            .collect();
        let prop_sets: String = obj_types
            .iter()
            .map(|t| {
                format!(
                    "<vim25:propSet><vim25:type>{}</vim25:type>{path_set}</vim25:propSet>",
                    xml_escape(t)
                )
            })
            .collect();

        let body = format!(
            r#"<vim25:RetrievePropertiesEx><vim25:_this type="PropertyCollector">propertyCollector</vim25:_this><vim25:specSet>{prop_sets}<vim25:objectSet><vim25:obj type="ContainerView">{view}</vim25:obj><vim25:skip>true</vim25:skip><vim25:selectSet xsi:type="vim25:TraversalSpec"><vim25:name>view</vim25:name><vim25:type>ContainerView</vim25:type><vim25:path>view</vim25:path><vim25:skip>false</vim25:skip></vim25:selectSet></vim25:objectSet></vim25:specSet><vim25:options/></vim25:RetrievePropertiesEx>"#
        );

        let mut out = Vec::new();
        let mut page = self.call(&body).await?;

        loop {
            let returnval = match page.find("returnval") {
                Some(r) => r,
                None => break, // no matching objects at all
            };
            for obj in returnval.children_named("objects") {
                out.push(ManagedObject::from_element(obj));
            }
            let token = returnval.child("token").map(|t| t.text.clone());
            let Some(token) = token.filter(|t| !t.is_empty()) else {
                break;
            };
            let cont = format!(
                r#"<vim25:ContinueRetrievePropertiesEx><vim25:_this type="PropertyCollector">propertyCollector</vim25:_this><vim25:token>{}</vim25:token></vim25:ContinueRetrievePropertiesEx>"#,
                xml_escape(&token)
            );
            page = self.call(&cont).await?;
        }

        Ok(out)
    }
}

/// One managed object plus the properties that were asked for.
#[derive(Debug, Clone)]
pub struct ManagedObject {
    pub moref: String,
    pub moref_type: String,
    /// Property path → the `<val>` element holding its value.
    pub props: Vec<(String, Element)>,
}

impl ManagedObject {
    /// Build from an `<objects>` element. Public so sheets can be unit-tested
    /// against captured vCenter XML without a live server.
    pub fn from_element(obj: &Element) -> Self {
        let (moref, moref_type) = obj
            .child("obj")
            .map(|o| (o.text.clone(), o.attr("type").unwrap_or_default().to_string()))
            .unwrap_or_default();

        let props = obj
            .children_named("propSet")
            .filter_map(|p| {
                let name = p.child("name")?.text.clone();
                let val = p.child("val")?.clone();
                Some((name, val))
            })
            .collect();

        Self { moref, moref_type, props }
    }

    pub fn prop(&self, name: &str) -> Option<&Element> {
        self.props.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Scalar property as text.
    pub fn str_prop(&self, name: &str) -> Option<String> {
        self.prop(name).map(|e| e.text.clone()).filter(|s| !s.is_empty())
    }

    pub fn i64_prop(&self, name: &str) -> Option<i64> {
        self.str_prop(name)?.parse().ok()
    }

    pub fn bool_prop(&self, name: &str) -> Option<bool> {
        match self.str_prop(name)?.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }

    /// A moref-valued property as (moref, declared managed-object type).
    ///
    /// vim25 sends `<val type="Datacenter" xsi:type="ManagedObjectReference">`:
    /// `xsi:type` only says "this is a reference", while the plain `type`
    /// attribute carries the actual type. Walking a `parent` chain needs the
    /// latter, and reading it beats inferring a type from the moref prefix.
    pub fn moref_prop(&self, name: &str) -> Option<(String, String)> {
        let el = self.prop(name)?;
        let moref = el.text.clone();
        if moref.is_empty() {
            return None;
        }
        Some((moref, el.attr("type").unwrap_or_default().to_string()))
    }

    /// Members of an array-valued property.
    ///
    /// vim25 names array elements after the property's declared *field type*,
    /// not the field name — `config.hardware.device` comes back as
    /// `<VirtualDevice xsi:type="VirtualDisk">`. Returning every child sidesteps
    /// having to know the type name, and callers filter on `xsi_type`.
    pub fn array_prop(&self, name: &str) -> Vec<&Element> {
        self.prop(name).map(|e| e.children.iter().collect()).unwrap_or_default()
    }
}
