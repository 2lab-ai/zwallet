const {WalletContractV4}=require('@ton/ton');
const {beginCell,internal,external,storeMessage,storeMessageRelaxed,storeStateInit}=require('@ton/core');
const {keyPairFromSeed}=require('@ton/crypto');
const crypto=require('crypto');
// Public test fixture only. Never output the seed, mnemonic, or private key.
const phrase='test test test test test test test test test test test junk';
let key=crypto.pbkdf2Sync(phrase,'mnemonic',2048,64,'sha512');
let x=crypto.createHmac('sha512','ed25519 seed').update(key).digest();
for(const i of [44,607,0]){const b=Buffer.alloc(37);b[0]=0;x.subarray(0,32).copy(b,1);b.writeUInt32BE(i+0x80000000,33);x=crypto.createHmac('sha512',x.subarray(32)).update(b).digest();}
const pair=keyPairFromSeed(x.subarray(0,32)); const wallet=WalletContractV4.create({workchain:0,publicKey:pair.publicKey});
const to='0:'+ '11'.repeat(32), amount=1234567890n,seqno=7,validUntil=1900000000;
const message=internal({to,value:amount,bounce:false});
const unsigned=beginCell().storeUint(698983191,32).storeUint(validUntil,32).storeUint(seqno,32).storeUint(0,8).storeUint(3,8).storeRef(beginCell().store(storeMessageRelaxed(message))).endCell();
const body=wallet.createTransfer({seqno,timeout:validUntil,sendMode:3,messages:[message],secretKey:pair.secretKey});
const ext=beginCell().store(storeMessage(external({to:wallet.address,init:wallet.init,body}),{forceRef:true})).endCell();
const fixture={sdk:{'@ton/ton':require('@ton/ton/package.json').version,'@ton/core':require('@ton/core/package.json').version,'@ton/crypto':require('@ton/crypto/package.json').version}, publicKey:pair.publicKey.toString('hex'),address:wallet.address.toRawString(),friendly:wallet.address.toString({bounceable:false}),codeHash:wallet.init.code.hash().toString('hex'),stateInitHash:beginCell().store(storeStateInit(wallet.init)).endCell().hash().toString('hex'),to,amountRaw:amount.toString(),seqno,validUntil,unsignedBoc:unsigned.toBoc({idx:false,crc32:false}).toString('hex'),unsignedHash:unsigned.hash().toString('hex'),signedBodyHash:body.hash().toString('hex'),signedBodyBoc:body.toBoc({idx:false,crc32:false}).toString('hex'),externalHash:ext.hash().toString('hex'),externalBoc:ext.toBoc({idx:false,crc32:false}).toString('hex')};
require('fs').writeFileSync(process.argv[2],JSON.stringify(fixture,null,2)+'\n');
