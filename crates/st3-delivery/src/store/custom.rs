//! The part of `store::custom` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use serde::{Deserialize, Serialize};
use st3_schema::custom::{self as schema, Authority, Basis, Expr, Manifest, Predicate, Select};
pub use st3_store::store::custom::*;
pub use st3_store::store::custom::{RegistrationRequest, attention_item, create_schema, flush, open, prepare, rebuild, source, typed};
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyRequest {
    pub subject: String,
    pub registration: String,
    pub revision: String,
    pub episode: String,
    pub fields: BTreeMap<String, Value>,
    pub actor: String,
    pub idempotency_key: String,
}

pub trait CustomStore {
    fn reply_custom_subject(&self, request: &ReplyRequest) -> Result<ClaimRecord, St3Error>;
    fn reply_custom_subject_authorized(&self, request: &ReplyRequest,
            authorization: Option<&super::assistant_write::AssistantWriteAuthorization>) -> Result<ClaimRecord, St3Error>;
}

impl CustomStore for Store {

    fn reply_custom_subject(&self, request: &ReplyRequest) -> Result<ClaimRecord, St3Error> {
        self.reply_custom_subject_authorized(request, None)
    }

    fn reply_custom_subject_authorized(&self, request: &ReplyRequest,
            authorization: Option<&super::assistant_write::AssistantWriteAuthorization>) -> Result<ClaimRecord, St3Error> {
        validate_actor(&request.actor)?;
        if request.idempotency_key.is_empty() || request.idempotency_key.len() > 256 {
            return Err(fail("reply needs a bounded idempotency key"));
        }
        let digest = canonical_hash(request).map_err(internal)?;
        let key = canonical_hash(&("custom.reply", &request.actor, &request.idempotency_key))
            .map_err(internal)?;
        self.connection.batched(|tx|->Result<ClaimRecord,St3Error>{
            if let Some(authorization) = authorization {
                super::assistant_write::authorize(tx, &self.origin, authorization)?;
            }
            let prior=claims(tx,&request.subject).map_err(internal)?.into_iter().filter(|c|c.body.pointer("/_custom_reply/key")==Some(&json!(key))).collect::<Vec<_>>();
            if let Some(c)=prior.first() {
                if prior.iter().any(|c|c.body.pointer("/_custom_reply/digest")!=Some(&json!(digest))) { return Err(St3Error::new("idempotency-conflict","reply key identifies competing requests")); }
                return Ok(c.clone());
            }
            flush(tx).map_err(internal)?;
            let view=source(tx,&request.subject).map_err(internal)?.ok_or_else(||fail("custom source is unavailable"))?;
            if view["registration"]!=request.registration||view["revision"]!=request.revision||view["state"]!="ready"||view["attention"]["active"]!=true||view["attention"]["episode"]!=request.episode {return Err(St3Error::new("stale-fence","custom source revision/episode changed"));}
            if view["attention"]["recipient"]!=request.actor {return Err(St3Error::new("forbidden","only the addressed person may reply"));}
            let (_,m,_)=registration(tx,&request.subject).map_err(internal)?.ok_or_else(||fail("registration disappeared"))?;
            let reply=&m.attention.as_ref().ok_or_else(||fail("no reply action"))?.reply;
            if request.fields.keys().any(|name|!reply.fields.contains_key(name)) { return Err(fail("reply contains undeclared or reserved input fields")); }
            schema::validate_fields(&schema::ClaimSchema{authority:Authority::Recipient,fields:reply.fields.clone(),additional_fields:false},&request.fields).map_err(typed)?;
            let history=claims(tx,&request.subject).map_err(internal)?.into_iter().filter(|c|fields(c).get("_registration")==Some(&json!(request.registration))).collect::<Vec<_>>();
            let slots=slots(&m,&history);
            let mut f=request.fields.clone();
            for(n,e)in &reply.bindings {f.insert(n.clone(),eval(e,&slots));}
            let input=ClaimInput{subject:request.subject.clone(),kind:reply.kind.clone(),actor:Some(request.actor.clone()),fields:f,evidence:vec![],expected_subject:None,idempotency_key:None};
            let f=prepare(tx,&input)?.ok_or_else(||fail("registration disappeared"))?;
            smallclaims::store::principals::rules_gate_tx(tx,&self.origin,&request.actor,&reply.kind,&request.subject).map_err(internal)?;
            let predecessor=latest_claim_id_tx(tx,&request.subject).map_err(internal)?.into_iter().collect::<Vec<_>>();
            let c=append_claim_tx(tx,&self.origin,&request.subject,&reply.kind,Some(&request.actor),&json!({"fields":f,"_custom_reply":{"revision":request.revision,"episode":request.episode,"key":key,"digest":digest}}),&predecessor,None).map_err(internal)?;
            flush(tx).map_err(internal)?;
            Ok(c)
        }).map_err(internal)?
    }
}

