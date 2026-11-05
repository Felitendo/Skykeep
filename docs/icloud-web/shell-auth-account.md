# Sign-in, session and account: static pass + live capture, 2026-10-19
Sign-in bundle: `~/.cache/quince-icloud-re/shell/pretty/idmsa/` (Apple's `idmsa.apple.com/appleauth` widget). All paths are under
`https://idmsa.apple.com/appleauth` (`urlContext`), POST JSON unless noted.

## Methods the web widget supports (VERIFIED-IN-CODE path strings)
| Method | Calls | In Quince? |
|---|---|---|
| Password (SRP-6a, worker `webSRPClientWorker.js`) | `/auth/signin/init` → `/auth/signin/complete` | yes |
| Code from a trusted device | `/auth/verify/trusteddevice/securitycode` | yes |
| SMS / phone | `/auth/verify/phone`, `/auth/verify/phone/securitycode` | yes |
| Trust this browser | `/auth/2sv/trust` (`/auth/2sv/donttrust`) | yes |
| **Passkey** (WebAuthn assertion) | `getDeviceChallange` → `POST /auth/verify/device/key/challenge {passkeyAutofill}`, then `verifyClientDeviceAssertion` → `POST /auth/verify/device/key?isRememberMeEnabled={bool}` with `{…assertion, rememberMe, accountName?, passkeyAutofill}`. **Seen live: this is how he signs in on the web.** | no |
| Security key (FIDO) | `/auth/verify/security/key`, `/auth/verify/securitykey` | no |
| Email code | `/auth/verify/email/securitycode`, `/auth/verify/email/resend` | no |
| Recovery key | `/auth/verify/recoverykey` | no |
| QR sign-in | `/signin/qrcode` | no |
| Account repair (terms, privacy) | `/auth/repair/complete`, `/auth/bridge/step/{n}` | no (shown as an error) |
| Other | `/auth/token/complete`, `/auth/push/complete`, `/auth/otp/{generate,validate}`, `/auth/escrow/{init,complete}`, `/auth/federate/*`, `/auth/obo`, `/auth/supportpin` | no |

## iCloud session (setup.icloud.com, seen live)
- `POST /setup/ws/1/accountLogin {dsWebAuthToken, accountCountryCode, extended_login}` returns dsInfo + webservices (+ `isWebAccessAllowed`).
- `POST /setup/ws/1/validate`: **421 means the session has expired**, so sign in again (seen live after an idle period).
- `POST {partition}-setup /setup/ws/1/storageUsageInfo`, `GET /setup/web/family/getFamilyDetails`.
- Plans: `gatewayws /acsegateway/v1/accounts/{dsid}/plans/icloud/pricing`, `…/web/plans/next-larger-plan`,
  `/v3/accounts/{dsid}/subscriptions/features/cloud.storage/plan-summary`.
- Shared web settings: `keyvalueservice /json/sync` (POSTed repeatedly).
- `logoutFromAllBrowsers` is referenced in the shell.

## Quince gaps worth doing
1. Passkey sign-in. Hard on Linux: WebKitGTK has no WebAuthn platform authenticator. A phone passkey would need the
   caBLE/hybrid QR flow. Low priority; password + 2FA already works.
2. Email-code and recovery-key fallbacks: small additions to quince-auth if an account has no trusted devices.
3. Repair (`bridge/step`): today an account with unaccepted iCloud terms just fails. Detect it and send the user to icloud.com.
