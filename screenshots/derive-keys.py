"""Derives the showcase addresses and xpub in showcase/world.ts from fixed seeds.

Well-formed (bech32 v0 with the hs prefix, base58check xpub) but belonging to nobody:
each 20-byte hash or key is a sha256 of a fixed string, not of any real key.

Run: python3 derive-keys.py
"""
import hashlib

CH="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def pm(v):
    g=[0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3];c=1
    for x in v:
        b=c>>25;c=(c&0x1ffffff)<<5^x
        for i in range(5): c^=g[i] if (b>>i)&1 else 0
    return c
def hrpx(h): return [ord(x)>>5 for x in h]+[0]+[ord(x)&31 for x in h]
def conv(d):
    acc=0;bits=0;r=[]
    for b in d:
        acc=(acc<<8)|b;bits+=8
        while bits>=5: bits-=5;r.append((acc>>bits)&31)
    if bits: r.append((acc<<(5-bits))&31)
    return r
def addr(h20):
    d=[0]+conv(h20);p=pm(hrpx("hs")+d+[0]*6)^1
    return "hs1"+"".join(CH[x] for x in d+[(p>>5*(5-i))&31 for i in range(6)])
def b58(b):
    A="123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";n=int.from_bytes(b,"big");s=""
    while n: n,r=divmod(n,58);s=A[r]+s
    return "1"*(len(b)-len(b.lstrip(b"\0")))+s
def h(s): return hashlib.sha256(s.encode()).digest()
for i in range(8): print("recv",i,addr(h("namehold-showcase-recv-%d"%i)[:20]))
print("lock",addr(h("namehold-showcase-lock")[:20]))
pub=b"\x02"+h("namehold-showcase-pub")
raw=bytes.fromhex("0488b21e")+b"\x03"+h("fp")[:4]+bytes.fromhex("80000000")+h("cc")+pub
print("xpub",b58(raw+hashlib.sha256(hashlib.sha256(raw).digest()).digest()[:4]))
