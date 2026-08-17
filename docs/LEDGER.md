# Ledger Hardware Wallet Support

Namehold supports Ledger Nano S, Nano S Plus, and Nano X for signing
transactions. Private keys never leave the device.

## Prerequisites

1. **Ledger firmware**: up to date (Ledger Live → Manager → check for updates).
2. **Handshake app**: install `ledger-app-hns` via Ledger Live (search
   "Handshake" in the app catalog). If not listed, you can sideload from
   [handshake-org/ledger-app-hns](https://github.com/handshake-org/ledger-app-hns).
3. **USB connection**: connect the device via USB (Bluetooth is not supported
   for the Handshake app).

## Import a Ledger wallet

1. Open Namehold → Wallets → **Connect a Ledger device**.
2. Enter a label and select the network (mainnet / testnet / regtest).
3. Make sure the device is unlocked and the Handshake app is open.
4. Click **Import from Ledger**. The device will prompt you to confirm the
   xpub export — approve it.
5. Namehold imports the account xpub and derives receive/change addresses
   locally. No secret is stored on disk.

## Signing transactions

When you send HNS or perform a name action (OPEN, BID, REVEAL, etc.), Namehold
builds the transaction locally and sends it to the device for signing:

1. The device shows the transaction details (outputs, amounts, covenant type,
   and name) on its screen.
2. Confirm each output on the device.
3. The signed transaction is returned to Namehold and broadcast.

No passphrase prompt appears — the device IS the signer.

## Technical details

- **Transport**: USB HID via `hidapi` (cross-platform). Vendor ID `0x2C97`.
- **Protocol**: APDU commands over 64-byte HID frames with `0x0101`/`0x05`
  framing (same as hsd-ledger).
- **Signing flow**: parse mode (whole-tx blob) → sign mode (per-input blob).
  The device verifies outputs during parse and returns DER signatures during
  sign.
- **Change detection**: Namehold tells the device which output is change (via
  `ChangeInfo` in the parse blob) so it doesn't prompt for change verification.
- **Name markers**: For the 7 name-bearing covenant types (REVEAL, REDEEM,
  REGISTER, UPDATE, RENEW, TRANSFER, REVOKE), Namehold appends a
  `LedgerCovenant` name marker so the device can display the human-readable
  name.

## Troubleshooting

### Device not found

**Symptom:** "Device not found" or "HID error" when importing or signing.

**Fix:**
1. Ensure the Ledger device is connected via USB (not Bluetooth).
2. Unlock the device with your PIN.
3. Open the Handshake app on the device.
4. Try again.

### Wrong app (0x6d00)

**Symptom:** Error message contains "wrong app" or status code `0x6d00`.

**Fix:**
1. On the device, navigate to the Handshake app and open it.
2. Retry the operation in Namehold.

### User rejected

**Symptom:** "User rejected" error or status code `0x6985`.

**Fix:**
1. This means you pressed the X button (reject) on the device during signing.
2. Retry the operation and press the checkmark (approve) on the device when prompted.

### Timeout

**Symptom:** "Timed out waiting for the Ledger" after 30 seconds.

**Fix:**
1. The device may have locked or the app closed during signing.
2. Unlock the device and open the Handshake app again.
3. Retry the operation.

### Covenant error (0x6a80)

**Symptom:** "Invalid data — covenant/tx serialization rejected" or status code `0x6a80`.

**Fix:**
1. This is rare and usually indicates a firmware mismatch.
2. Ensure your Ledger firmware is up to date (1.6.0 or later).
3. Reinstall the Handshake app on the device.
4. If the problem persists, report it with the transaction details.

## Limitations

- Only account index 0 is supported (standard BIP44 path `m/44'/5353'/0'`).
- The change address is always branch 1, index 0 (first change address).
- Bluetooth is not supported (USB only).
- The device must stay connected and the Handshake app open throughout signing.

## Developer Testing: In-Process Simulator

For UI testing and development, Namehold includes an in-process Ledger device
simulator. It lets you click through every UX path (import, signing, errors) and
test failure scenarios without a physical device or Docker.

### Building with the simulator

```bash
cargo tauri dev --features mock-ledger
```

### Running scenarios

Set the `NAMEHOLD_LEDGER_SIM` environment variable to select a scenario:

```bash
# Happy path — import and signing succeed
NAMEHOLD_LEDGER_SIM=happy cargo tauri dev --features mock-ledger

# No device found
NAMEHOLD_LEDGER_SIM=no_device cargo tauri dev --features mock-ledger

# Wrong app open (0x6d00)
NAMEHOLD_LEDGER_SIM=wrong_app cargo tauri dev --features mock-ledger

# Device locked (0x5515)
NAMEHOLD_LEDGER_SIM=locked cargo tauri dev --features mock-ledger

# User rejects on device (0x6985)
NAMEHOLD_LEDGER_SIM=reject cargo tauri dev --features mock-ledger

# Timeout waiting for device approval (31s hang)
NAMEHOLD_LEDGER_SIM=timeout cargo tauri dev --features mock-ledger

# Device disconnects mid-signing
NAMEHOLD_LEDGER_SIM=disconnect cargo tauri dev --features mock-ledger
```

### What to test

**Import path:**

- Click "Wallets" → "Connect a Ledger device"
- Enter a label and select a network
- Click "Import from Ledger"
- Observe: success toast, error message, or timeout handling

**Signing path:**

- Import a Ledger wallet (using `happy` mode)
- Click "Send" or perform a name action (OPEN, BID, REVEAL, etc.)
- Click "Sign & Broadcast"
- Observe: button label ("Confirm on your Ledger…"), success/error toast, or state recovery after failure

**Error messages:**

- Each scenario produces a distinct error message (from the backend, mapped by the frontend)
- Verify the message is actionable and not clobbered by generic fallbacks
- Examples: "Timed out — approve or reject the prompt on your Ledger device", "Your Ledger is locked — unlock the device and try again"

### Notes

- The simulator is **dev-only** and never compiled into release builds.
- Each scenario is deterministic and repeatable — you can iterate on UX copy without restarting.
- The simulator responds with valid APDU frames, so the transport layer is exercised end-to-end.
- For high-fidelity crypto testing (verifying signatures against real firmware), use a physical device or Speculos (future enhancement).
