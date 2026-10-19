# switch-profile

Make another regtest profile the active one and get its signer usable.

- **Parameters:** the target profile (a role from [environment](../environment.md#profiles)).
- **Blocks mined:** 0.

## Steps

1. On the Wallet page, pick the target profile in the profile selector of the header. Never pick a non-regtest profile ([rules](../rules.md)).
2. Wait until the Wallet page shows the target profile's balance and Owned Names.
3. The page shows `Signer locked` right after the switch: activating a profile drops a signer unlocked for another profile. `Signer unlocked` there, or a signing refused with "the unlocked signer is for a different wallet profile", is ❌ with a snapshot. If the case signs, press `Unlock` and enter the target profile's passphrase in the secure window.
4. Press **Sync** if the case reads chain state in the new profile.

## End state

The target profile is active, its signer locked or unlocked as the case needs, and synced.
