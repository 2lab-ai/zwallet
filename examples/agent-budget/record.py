#!/usr/bin/env python3
"""Record real zwallet commands against an isolated Anvil chain; no public funds.

Build with `cargo build --locked` first. Requires a debug binary for test-only daemon
scoping. Stdlib only. Passwords are random, held in memory, sent on stdin, never logged.
This is a scripted agent workflow, not a recording of an LLM making decisions.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.request
from decimal import Decimal

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/zwallet')
    parser.add_argument('--anvil', default='anvil')
    parser.add_argument('--output', type=Path, default=ROOT/'docs/assets')
    args = parser.parse_args()
    binary = args.binary.resolve()
    # Release binaries ignore the daemon scope; do not let this fixture lock real sessions.
    if binary != (ROOT/'target/debug/zwallet').resolve():
        parser.error('use this checkout\'s target/debug/zwallet (cargo build --locked)')
    if not binary.exists():
        parser.error('run cargo build --locked first')
    # Compile profile is also checked by inspecting the debug-only scope string.
    if b'ZWALLET_TEST_DAEMON_SCOPE' not in binary.read_bytes():
        parser.error('binary lacks debug-only test daemon scope; refusing to run')
    args.output.mkdir(parents=True, exist_ok=True)
    if os.uname().sysname == 'Darwin':
        subprocess.run(['codesign','--force','--sign','-',str(binary)],check=True,capture_output=True)
    home = Path(tempfile.mkdtemp(prefix='zwt-demo-',dir='/tmp'))
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0)); port = sock.getsockname()[1]
    rpc_url = f'http://127.0.0.1:{port}'
    env = {'PATH':os.defpath,'ZWALLET_HOME':str(home),
           'ZWALLET_TEST_DAEMON_SCOPE':str(home), 'ZWALLET_RPC_SEPOLIA':rpc_url}
    password = secrets.token_urlsafe(32)
    anvil = None
    evidence = {'network':'local Anvil, Sepolia chain ID; NOT public Sepolia',
                'sourceRevision': subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
                'scriptedWorkflow':True, 'presentation':'selected JSON fields; commands executed for real',
                'commands':[], 'scenes':[]}
    cast = [json.dumps({'version':2,'width':104,'height':24,
                       'title':'zwallet: allocate, execute, verify, report (local Anvil)',
                       'env':{'TERM':'xterm-256color'}})]
    started = time.monotonic()
    def emit(s):
        # A terminal needs both carriage return and line feed for every new line.
        s = s.replace('\r\n', '\n').replace('\n', '\r\n')
        cast.append(json.dumps([round(time.monotonic()-started,3),'o',s]))
        print(s,end='',flush=True)
    def scene(label):
        evidence['scenes'].append({'at':round(time.monotonic()-started,3),'label':label})
        emit('\x1b[2J\x1b[H'+label+'\r\n'+'-'*86+'\r\n')
    def rpc(method,params):
        req=urllib.request.Request(rpc_url,json.dumps({'jsonrpc':'2.0','id':1,'method':method,'params':params}).encode(),{'Content-Type':'application/json'})
        with urllib.request.urlopen(req,timeout=10) as response: result=json.load(response)
        if 'error' in result: raise RuntimeError(f'local RPC {method} failed')
        return result['result']
    def run(cmd,fields=None,secret=False,visible=False,expected=0):
        full=[str(binary),*cmd,'--json']
        if secret: full.append('--password-stdin')
        if visible:
            emit('$ zwallet '+' '.join(full[1:])+'\r\n')
        p=subprocess.run(full,input=(password+'\n') if secret else None,
                         env=env,capture_output=True,text=True,timeout=35)
        if password in p.stdout+p.stderr: raise RuntimeError('secret detected; refusing to save recording')
        value=json.loads(p.stdout)
        evidence['commands'].append({'argv':full[1:],'exitCode':p.returncode,'result':value})
        if p.returncode != expected:
            raise RuntimeError(f'{cmd[0]}: unexpected exit {p.returncode}: {value.get("errorCode", "unknown")}')
        if visible:
            selected={k:value[k] for k in fields if k in value} if fields else value
            emit(json.dumps(selected,indent=2)+'\r\n')
            time.sleep(3)
        return value
    try:
        anvil=subprocess.Popen([args.anvil,'--host','127.0.0.1','--port',str(port),
                                '--chain-id','11155111','--silent'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for _ in range(100):
            try:
                assert int(rpc('eth_chainId',[]),16)==11155111
                break
            except Exception:
                if anvil.poll() is not None: raise RuntimeError('Anvil exited')
                time.sleep(.1)
        else: raise RuntimeError('local Anvil did not start')
        # Human provisioning, automated only for the disposable demonstration.
        created=run(['create','demo'],secret=True)
        address=created['address']
        recipient='0x1111111111111111111111111111111111111111'
        run(['whitelist','add','dev-service',recipient,'--chains','sepolia'],secret=True)
        run(['whitelist','native-cap','sepolia','0.005'],secret=True)
        run(['whitelist','fee-cap','sepolia','0.001'],secret=True)
        rpc('anvil_setBalance',[address,hex(20_000_000_000_000_000)])
        evidence['provisioning']={'initialWei':'20000000000000000','nativeCap':'0.005 ETH per transaction',
                                 'recipient':recipient,'funding':'anvil_setBalance; valueless local ETH'}
        scene('01 / HUMAN: fund a dedicated wallet; unlock one session')
        emit('LOCAL ANVIL / valueless ETH / automated disposable setup\r\n')
        emit('Allocation: 0.020 ETH | payee: @dev-service | per-tx cap: 0.005 ETH\r\n')
        run(['unlock','30m'],['locked','duration','verified','verifiedBy'],secret=True,visible=True)
        scene('02 / AGENT WORKFLOW: inspect funds and preview the payment')
        run(['balance','--chain','sepolia'],['chain','symbol','balance','verified'],visible=True)
        preview=run(['transfer','--chain','sepolia','--to','@dev-service','--amount','0.001','--dry-run'],
                    ['dryRun','amount','sufficientFunds','whitelist'],visible=True)
        assert preview['dryRun'] and preview['sufficientFunds']
        scene('03 / AGENT WORKFLOW: send and verify, without another password')
        sent=run(['transfer','--chain','sepolia','--to','@dev-service','--amount','0.001','--wait'],
                 ['amount','txHash','status','success','gasUsed'],visible=True)
        assert sent['success'] is True
        checked=run(['tx','--chain','sepolia',sent['txHash']],['found','status','success','gasUsed'],visible=True)
        assert checked['success'] is True
        receipt=rpc('eth_getTransactionReceipt',[sent['txHash']])
        assert int(rpc('eth_getBalance',[recipient,'latest']),16)==1_000_000_000_000_000
        scene('04 / POLICY: an over-limit request is refused before signing')
        before=rpc('eth_getTransactionCount',[address,'latest'])
        denied=run(['transfer','--chain','sepolia','--to','@dev-service','--amount','0.006','--wait'],
                   ['errorCode','error'],visible=True,expected=1)
        assert denied['errorCode']=='WHITELIST_DENIED'
        assert rpc('eth_getTransactionCount',[address,'latest'])==before
        scene('05 / REPORT: account for the spend, fee and remaining allocation')
        after=run(['balance','--chain','sepolia'],['symbol','balance'],visible=True)
        gas=int(receipt['gasUsed'],16)*int(receipt['effectiveGasPrice'],16)
        remaining=int(after['raw'])
        assert remaining + gas + 1_000_000_000_000_000 == 20_000_000_000_000_000
        report={'task':'local development payment fixture','payment':'0.001 ETH',
                'gasFeeETH':str(Decimal(gas)/Decimal(10**18)),
                'remainingETH':str(Decimal(remaining)/Decimal(10**18)),
                'confirmedTransactions':1,'deniedRequests':1,'txHash':sent['txHash'],
                'provenance':'example script combines CLI JSON and local RPC receipt; not a built-in report command'}
        evidence['report']=report
        emit('# Example script report (computed; NOT a built-in zwallet command)\r\n')
        emit(f'Confirmed: 1 | spent: {report["payment"]} | fee: {report["gasFeeETH"]} ETH\r\n')
        emit(f'Remaining: {report["remainingETH"]} ETH | over-limit request: refused\r\n')
        time.sleep(4)
        scene('06 / HUMAN CONTROL: end the session')
        locked=run(['lock'],['locked','wasUnlocked'],visible=True)
        assert locked['locked']
        evidence['duration']=round(time.monotonic()-started,3)
        evidence['binaryVersion']=subprocess.check_output([str(binary),'--version'],env=env,text=True).strip()
        evidence['anvilVersion']=subprocess.check_output([args.anvil,'--version'],text=True).splitlines()[0]
        (args.output/'agent-budget.cast').write_text('\n'.join(cast)+'\n')
        (args.output/'agent-budget-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
    finally:
        # Scope limits the debug binary's lock to this fixture only, on success and failure.
        subprocess.run([str(binary),'lock','--json'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=15)
        if anvil:
            anvil.terminate()
            try: anvil.wait(timeout=5)
            except subprocess.TimeoutExpired: anvil.kill(); anvil.wait()
        shutil.rmtree(home)

if __name__=='__main__':
    # Tests intentionally lock all /tmp/zwt-* sessions. Share their cross-process
    # serialization protocol so neither this recording nor the tests are interrupted.
    with open(f'/tmp/zwt-serial-{os.geteuid()}.lock', 'a+') as serial:
        fcntl.flock(serial, fcntl.LOCK_EX)
        main()
