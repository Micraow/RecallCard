"""Explicit opt-in local MCP smoke against invented originals; no model or API.

Uses an existing binary, never builds it. Output contains synthetic data only.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile

from bounded_span_adapter import BoundedSpanWorkflow, wire_packet_hash
from judgment_contract import Assessment, BudgetLedger, Judgment, Scope, encode


def run(binary, output):
    output.mkdir(exist_ok=False)
    ledger = BudgetLedger()
    records = []
    with tempfile.TemporaryDirectory(prefix="toy-", dir=output) as directory:
        root = Path(directory)
        vault = root / "vault"
        env = {**os.environ, "RECALLCARD_STATE_DIR":str(root / "state")}
        subprocess.run([binary,"--vault",str(vault),"init"],env=env,check=True,capture_output=True)
        for i, role in enumerate(("user","assistant")):
            event={"role":role,"origin":"native","scope":"personal","session_id":"adapter-toy",
                "occurred_at":"2026-01-02T00:00:00Z","content":
                    "beacon "+"An invented constraint α. "*200 if i==0 else "A later invented reply.",
                "source":{"platform":"adapter-toy","account_namespace":"synthetic-owner",
                          "conversation_id":"toy-session","message_id":str(i)},
                "metadata":{"qwen":{"node_id":str(i),"parent_id":None if i==0 else "0","on_current_path":True}}}
            source=root/f"event-{i}.json";source.write_bytes(encode(event))
            subprocess.run([binary,"--vault",str(vault),"capture","--file",str(source)],
                           env=env,check=True,capture_output=True)
        proc=subprocess.Popen([binary,"--vault",str(vault),"mcp","--scope","personal"],
                              stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env)
        sequence=0
        def rpc(method,params):
            nonlocal sequence
            sequence+=1
            request={"jsonrpc":"2.0","id":sequence,"method":method,"params":params}
            proc.stdin.write(encode(request)+b"\n");proc.stdin.flush()
            selector=selectors.DefaultSelector();selector.register(proc.stdout,selectors.EVENT_READ)
            assert selector.select(15),"local MCP timeout";selector.close()
            wire=proc.stdout.readline();response=json.loads(wire)
            records.append({"request":request,"response":response,"wire_bytes":len(wire)})
            assert response.get("id")==sequence and "error" not in response,response
            if method!="tools/call":return response["result"]
            result=response["result"];assert not result.get("isError"),result
            assert len(result["content"])==1 and result["content"][0]["type"]=="text"
            return result["content"][0]["text"].encode()
        try:
            rpc("initialize",{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"synthetic-adapter-smoke","version":"1"}})
            notification={"jsonrpc":"2.0","method":"notifications/initialized"}
            proc.stdin.write(encode(notification)+b"\n");proc.stdin.flush()
            records.append({"request":notification,"response":None,"wire_bytes":0})
            scope=Scope.create("beacon",["personal"],{"role":"user"},"2026-02-03T00:00:00Z")
            ticket=ledger.reserve_tool(32768)
            raw=rpc("tools/call",{"name":"search","arguments":{"query":scope.query,"target":"events",
                "detail":"session_discovery_compact","limit":3,"budget_bytes":32768,
                "event_filter":scope.payload()["event_filter"],"as_of":scope.as_of}})
            workflow=BoundedSpanWorkflow.from_discovery(scope,raw,ledger,ticket)
            packet=workflow.initial_packet();candidate=packet["candidates"][0];cid=candidate["candidate_id"]
            support=(candidate["evidence"][0]["evidence_id"],)
            workflow.accept(Judgment(packet["packet_id"],(Assessment(cid,"uncertain","partial",support),),
                                     None,(cid,),"need_inspection"))
            ticket=workflow.begin_inspection(cid,4096)
            wire=workflow._wire_candidates[cid];template=workflow._wire_template
            args={"refs":[wire["anchor_ref"]],"context_anchor_ref":wire["anchor_ref"],"include_adjacent":True,
                "detail":"compact","budget_bytes":4096,"event_filter":template["event_filter"],
                "as_of":template["as_of"],"offset_bytes":wire["evidence"][0]["end_byte"],
                "context_review":{"stage":"inspect","template":template,"candidate":wire,
                    "assessment":{"decision":"uncertain","selected_anchor_ref":None,"evidence_indices":[]}}}
            raw=rpc("tools/call",{"name":"read","arguments":args})
            workflow.consume_inspection(ticket,raw,wire["anchor_ref"])
            packet=workflow.next_packet();candidate=packet["candidates"][0]
            support=(candidate["evidence"][-1]["evidence_id"],)
            workflow.accept(Judgment(packet["packet_id"],(Assessment(cid,"relevant","partial",support),),
                                     cid,(),"selected"))
            args=workflow.selected_read_args();ticket=ledger.reserve_tool(8192)
            raw=rpc("tools/call",{"name":"read","arguments":args});ledger.finish_tool(ticket,raw)
            result=json.loads(raw)
            assert result.get("results") and result.get("deep_read_performed") is not False,result
            # Rehashing forged text cannot pass the original server evidence gate.
            proof=args["context_review"];span=proof["candidate"]["evidence"][0]
            span["text"]="X"+span["text"][1:]
            proof["candidate"]["packet_hash"]=wire_packet_hash(proof["template"],proof["candidate"])
            ticket=ledger.reserve_tool(8192);raw=rpc("tools/call",{"name":"read","arguments":args})
            ledger.finish_tool(ticket,raw);denied=json.loads(raw)
            assert denied.get("deep_read_performed") is False and denied.get("results")==[],denied
            receipt={"schema":"recallcard.bounded-adapter-synthetic-mcp/1","passed":True,
                "existing_binary_sha256":hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                "synthetic_only":True,"extra_reads_for_full_original_verification":0,
                "unbounded_original_fetches":0,"model_or_external_api_calls":0,
                "rust_builds":0,"tool_calls":ledger.tool_calls,"result_bytes":ledger.result_bytes,
                "judge_rounds_without_model":ledger.rounds,"partial_answer_source_selected":True,
                "real_server_rejected_rehashed_forged_text":True,"ledger":ledger.snapshot()}
            (output/"receipt.json").write_bytes(encode(receipt)+b"\n")
            (output/"synthetic-wire.json").write_bytes(encode(records)+b"\n")
            print(json.dumps(receipt))
        finally:
            (output/"synthetic-wire.json").write_bytes(encode(records)+b"\n")
            proc.stdin.close();proc.wait(timeout=15)
            stderr=proc.stderr.read();assert proc.returncode==0 and not stderr,(proc.returncode,stderr)


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary",required=True);parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args();run(args.binary,args.output)
