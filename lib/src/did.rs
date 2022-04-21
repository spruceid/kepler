//use async_trait::async_trait;
//use didkit::DID_METHODS as DMS;
//use did_method_key::DIDKey;
//use ssi::{
//    did::{DIDMethods, Document},
//    did_resolve::{DIDResolver, DocumentMetadata, ResolutionInputMetadata, ResolutionMetadata},
//};
//
//pub fn did_resolver() -> DIDMethods<'static> {
//    let mut dms = DIDMethods::default();
//    dms.insert(&DIDKey);
//    dms
//}
//
//#[async_trait(?Send)]
//impl DIDResolver for DID_METHODS {
//    async fn resolve(
//        &self,
//        did: &str,
//        input_metadata: &ResolutionInputMetadata,
//    ) -> (
//        ResolutionMetadata,
//        Option<Document>,
//        Option<DocumentMetadata>,
//    ) {
//        DMS.to_resolver().resolve(did, input_metadata).await
//    }
//}
