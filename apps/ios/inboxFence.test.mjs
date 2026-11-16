
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { sendFences } from "./sendFence.ts";
import { submitAnswer } from "./answer.ts";
import { ClientError, retryTransient, notApplied } from "../../clients/typescript/fractal-client/index.ts";
const current=readFileSync("store.tsx","utf8");
const take=(source,start,end)=>{const a=source.indexOf(start),b=source.indexOf(end,a);assert(a>=0&&b>a);return source.slice(a,b);};
const methods={done:["    async done(", "    /** Clear an item"],markRead:["    async markRead(", "    /** Change the person"],respondPrompt:["    async respondPrompt(", "    /** An image"]};
function harness(source,method,{held="snapshot/harbor/12/digest",refuseFirst=false}={}) {
 const trace=[];let sequence=0,sendCount=0;
 const revisions={"attention/example":"revision/viewed"};
 const client={};
 const send=async request=>{trace.push(["send",request]);assert.deepEqual(request.fence.subject_revisions,revisions); if(refuseFirst&&sendCount++===0)throw new ClientError({api_version:"st3.client.v0",error_version:"st3.client.error.v0",request_id:"request/stale",code:"stale-fence",message:"snapshot belongs to another host",retryable:false,details:{applied:"none"}},409);return {};};
 for(const name of ["workDone","messageRead","promptRespond"])client[name]=send;
 client.attentionGet=async()=>({value:{kind:"attention",state:"open"}});
 const fence=async(revs={})=>{trace.push(["capabilities"]);return {snapshot_id:"snapshot/harbor/20/new",subject_revisions:revs};};
 const deps={client,acted:{current:new Set()},personAnswer:()=>undefined,setError:()=>{},status:"online",proof:undefined,setBusy:()=>{},quality:{answerSent:()=>0,answerConfirmed:()=>{}},submitAnswer,sendFences,lastSnapshot:{current:held},fence,actionId:()=>`action/${++sequence}`,ClientError,errorText:e=>e.message,loadLists:async()=>{},promptAnswers:()=>({target_id:"prompt/example",episode:"episode/1",answers:["accept"]}),promptGone:()=>false,notApplied};
 const runSrc=stripTypeScriptTypes(take(source,"  async function runAction(","  async function runAvailabilityWrite("));
 deps.runAction=new Function("status","proof","setError","setBusy","retryTransient","notApplied","loadLists","errorText",`${runSrc};return runAction;`)(deps.status,deps.proof,deps.setError,deps.setBusy,retryTransient,notApplied,deps.loadLists,deps.errorText);
 const [start,end]=methods[method]; const methodSrc=stripTypeScriptTypes(`({${take(source,start,end)}})`);
 const action=new Function(...Object.keys(deps),`return (${methodSrc}).${method};`)(...Object.values(deps));
 return {trace,run:()=>action({id:"attention/example",revision:"revision/viewed",source_id:"step/example",episode:"episode/1"},method==="respondPrompt"?"accept":"Read")};
}
for(const method of Object.keys(methods)){
 for(const held of ["snapshot/harbor/12/digest",""]){
  const h=harness(current,method,{held});assert.equal(await h.run(),true);assert.equal(h.trace.filter(x=>x[0]==="capabilities").length,held?0:1);assert.equal(h.trace.filter(x=>x[0]==="send").length,1);
 }
 const switched=harness(current,method,{held:"snapshot/foreign/12/digest",refuseFirst:true});assert.equal(await switched.run(),true);assert.equal(switched.trace.filter(x=>x[0]==="capabilities").length,1);assert.equal(switched.trace.filter(x=>x[0]==="send").length,2);assert.equal(switched.trace.filter(x=>x[0]==="send")[1][1].fence.snapshot_id,"snapshot/harbor/20/new");
}
console.log("Actual store callback regression checks: 3 methods × held/missing/foreign snapshot passed; viewed subject revision retained through retry.");
