import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ClientError } from '../../clients/typescript/fractal-client/index.ts';
import { submitAnswer } from './answer.ts';
import { AttentionDeliveryTracker, retainDeliveryPrincipal } from './attentionDelivery.ts';
test('unknown keeps exact request and blocks opposite answer until explicit same-key check', async () => {
 const tracker=new AttentionDeliveryTracker(), original={id:'action/original',idempotency_key:'original'},sent=[];let builds=0,connected=false,cleared=0;
 const ports={build:async()=>{builds++;return original;},send:async request=>{sent.push(request);if(!connected)throw new TypeError('Network request failed');return {value:{kind:'action-result',status:'completed',action_id:request.id}};},stillWaiting:async()=>true,wait:async()=>{}};
 const result=await tracker.run('attention/item',async()=>tracker.accept('attention/item',await submitAnswer(ports),()=>{cleared++;}));
 assert.equal(result,false);assert.equal(tracker.snapshot().delivery['attention/item'],'unconfirmed');
 let opposite=0;assert.equal(await tracker.run('attention/item',async()=>{opposite++;return true;}),false);assert.equal(opposite,0);
 assert.equal(await tracker.check('attention/item'),false);assert.equal(builds,1);assert(sent.every(request=>request===original));assert.equal(cleared,0);
 connected=true;assert.equal(await tracker.check('attention/item'),true);assert.equal(cleared,1);assert.equal(tracker.snapshot().delivery['attention/item'],'confirmed');assert.equal(builds,1);assert(sent.every(request=>request===original));
});
test('old connection cannot check retained writes or erase drafts after profile change', async()=>{
 const tracker=new AttentionDeliveryTracker();let check=0,cleared=0;
 await tracker.accept('attention/item',{ok:false,how:'unconfirmed',error:new Error(),check:async()=>{check++;return {ok:true,how:'sent'};}},()=>{cleared++;});
 tracker.close();assert.equal(await tracker.check('attention/item'),false);assert.equal(check,0);assert.equal(cleared,0);
});
test('unclassified send errors cannot authorize a fresh opposite answer', async()=>{
 const tracker=new AttentionDeliveryTracker();const original={id:'original'};let built=0,sent=0;
 const error=new Error('unexpected decoder failure after send');
 const outcome=await submitAnswer({build:async()=>{built++;return original;},send:async request=>{assert.equal(request,original);sent++;throw error;},stillWaiting:async()=>true,wait:async()=>{}});
 assert.equal(outcome.how,'unconfirmed');await tracker.accept('attention/item',outcome);
 assert.equal(await tracker.run('attention/item',async()=>true),false);
 assert.equal(sent,1,'an unclassified local error is not automatically retried');
 await tracker.check('attention/item');assert.equal(built,1);assert.equal(sent,2);
});

test('socket capability loss preserves identity while authenticated-client or principal changes replace it',()=>{
 const client={},otherClient={};
 const first=retainDeliveryPrincipal({client:null,actor:undefined},client,'person/example/session/a');
 const offline=retainDeliveryPrincipal(first,client,undefined);
 assert.equal(offline.actor,first.actor);
 assert.equal(retainDeliveryPrincipal(offline,otherClient,undefined).actor,undefined);
 assert.equal(retainDeliveryPrincipal(offline,client,'person/other/session/b').actor,'person/other/session/b');
});
test('a later replay refusal cannot erase original ambiguity or unlock a new action',async()=>{
 const tracker=new AttentionDeliveryTracker(),original={id:'original'};let sends=0,builds=0;
 const stale=new ClientError({api_version:'st3.client.v0',error_version:'st3.client.error.v0',request_id:'stale',code:'stale-fence',message:'snapshot moved',retryable:false,details:{applied:'none'}},409);
 const ports={build:async()=>{builds++;return original;},send:async request=>{assert.equal(request,original);if(sends++===0)throw new TypeError('Network request failed');throw stale;},stillWaiting:async()=>true,wait:async()=>{}};
 const outcome=await submitAnswer(ports);assert.equal(outcome.how,'unconfirmed');assert.equal(builds,1,'automatic same-request refusal cannot create a new key');
 await tracker.accept('attention/item',outcome);await tracker.check('attention/item');
 assert.equal(tracker.snapshot().delivery['attention/item'],'unconfirmed');assert.equal(await tracker.run('attention/item',async()=>true),false);assert.equal(builds,1);
});
test('authoritative closure settles review but does not prove delivery or erase saved drafts',async()=>{
 const tracker=new AttentionDeliveryTracker();let closed=false,cleanup=0;
 const outcome=await submitAnswer({build:async()=>({id:'original'}),send:async()=>{throw new TypeError('Network request failed');},stillWaiting:async()=>!closed,wait:async()=>{}});
 await tracker.accept('attention/item',outcome,()=>{cleanup++;});closed=true;
 assert.equal(await tracker.check('attention/item'),true);
 assert.equal(tracker.snapshot().delivery['attention/item'],'settled');assert.equal(cleanup,0);
});
test('terminal delivery belongs to its originating revision, but uncertainty guards every revision',async()=>{
 const tracker=new AttentionDeliveryTracker();
 await tracker.run('attention/item',async()=>tracker.accept('attention/item',{ok:true,how:'already'}),false,'v1');
 assert.equal(tracker.forRevision('attention/item','v1'),'settled');assert.equal(tracker.forRevision('attention/item','v2'),undefined);
 await tracker.run('attention/item',async()=>tracker.accept('attention/item',{ok:true,how:'sent'}),false,'v2');
 assert.equal(tracker.forRevision('attention/item','v1'),undefined);assert.equal(tracker.forRevision('attention/item','v2'),'confirmed');
 await tracker.run('attention/item',async()=>tracker.accept('attention/item',{ok:false,how:'unconfirmed',error:new Error(),check:async()=>({ok:false,how:'unconfirmed',error:new Error(),check:async()=>({ok:true,how:'sent'})})}),false,'v3');
 assert.equal(tracker.forRevision('attention/item','v4'),'unconfirmed');
 let sent=false;assert.equal(await tracker.run('attention/item',async()=>{sent=true;return true;},false,'v4'),false);assert.equal(sent,false);
 assert.equal(tracker.snapshot().revisions['attention/item'],'v3','a blocked new revision cannot relabel the original request');
});
