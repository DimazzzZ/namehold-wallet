import { Card } from "./ui/Card";
import { Button } from "./ui/Button";
import { openExternal } from "../lib/openExternal";
import logo from "../../src-tauri/icons/logo.png";

const APP_VERSION = "0.3.0";

export function AboutPage() {
  const handleGitHubClick = async () => {
    await openExternal("https://github.com/DimazzzZ/namehold-wallet/issues");
  };

  return (
    <div className="flex items-center justify-center min-h-full">
      <Card className="w-full max-w-md shadow-lg">
        <div className="flex flex-col items-center gap-6 p-8">
          {/* Logo */}
          <img src={logo} alt="Namehold" className="h-24 w-24 rounded-lg" />

          {/* App name */}
          <h1 className="text-3xl font-bold text-gray-900">Namehold</h1>

          {/* Version */}
          <div className="text-sm text-gray-500">Version {APP_VERSION}</div>

          {/* Description */}
          <p className="text-center text-sm text-gray-600 leading-relaxed">
            A self-custodial Handshake TLD manager. Bid, register, renew, and
            manage your Handshake domains.
          </p>

          {/* GitHub link */}
          <Button
            variant="primary"
            onClick={handleGitHubClick}
            className="w-full"
          >
            Report Issues or Request Features
          </Button>

          {/* Footer note */}
          <div className="text-xs text-gray-400 text-center">
            Built for the Handshake community
          </div>
        </div>
      </Card>
    </div>
  );
}
