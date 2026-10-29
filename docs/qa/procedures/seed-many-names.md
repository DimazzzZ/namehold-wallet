# seed-many-names

Gives the seller profile **210 registered names** in one auction cycle of the hsd wallet, then transfers them to the app. The batch cases need more than 100 names (one batch transaction) and more than 200 (one capability request of the backend). `W`, `mine`, `info` and `TAG` are the helpers of [environment](../environment.md#regtest-node-and-hsd-wallet).

- **Blocks mined:** about 36.
- **Needs:** about 20 HNS spendable in the hsd wallet (`W getbalance`; else `mine 10`, then `mine 2`).
- **Names:** `qn000$TAG` … `qn209$TAG` (role **NM**, "the many names"). The report records `TAG`.

Each step goes out in batches of 100 actions per transaction (`sendbatch`): one transaction per name would chain more unconfirmed transactions than the mempool accepts.

```bash
NM=$(python3 -c "print(' '.join(f'qn{i:03d}$TAG' for i in range(210)))")
batch() {  # batch '<json action template with NAME>' — sends the names in chunks of 100
  python3 - "$1" $NM <<'PY' | while read -r chunk; do W sendbatch "$chunk" >/dev/null || echo "sendbatch failed"; done
import json, sys
tpl, names = sys.argv[1], sys.argv[2:]
for i in range(0, len(names), 100):
    print(json.dumps([json.loads(tpl.replace("NAME", n)) for n in names[i:i + 100]]))
PY
}
batch '["OPEN","NAME"]'; mine 6
info qn000$TAG; info qn209$TAG                                   # BIDDING, else mine 1
batch '["BID","NAME",0.01,0.02]'; mine 6                          # REVEAL, else mine 1
W sendbatch '[["REVEAL"]]' >/dev/null; mine 11                    # CLOSED, else mine 1
batch '["UPDATE","NAME",{"records":[]}]'; mine 1
APP=<the seller's Receive Address, rs1…>
batch "[\"TRANSFER\",\"NAME\",\"$APP\"]"; mine 11
batch '["FINALIZE","NAME"]'; mine 1
info qn000$TAG; info qn209$TAG                                   # owner: the app's address, transfer 0
```

Then **Sync** in the seller profile and open Names › `Owned`: the chip counts the 210 names plus whatever the profile already had. If `REVEAL` without a name reveals nothing (`W sendbatch` answers an error), run `W sendreveal` once instead.

**End state:** 210 more names in the seller profile, each with a 0.01 HNS value. They stay for later runs; a case that transfers them away says where they go.
