import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { Select } from "./ui/Select";
import { Button } from "./ui/Button";
import { useAssets } from "../queries/assets";

interface ResourceRecord {
  type: string;
  ns?: string;
  address?: string;
  txt?: string[];
  hash?: number;
  algorithm?: number;
  digestType?: number;
  digest?: string;
}

interface NameInfo {
  name?: string;
  state?: string;
  height?: number;
  renewal?: number;
  stats?: {
    daysUntilExpire?: number;
    blocksUntilExpire?: number;
  } | null;
  data?: {
    records?: ResourceRecord[];
  } | null;
}

function RecordTable({ records }: { records: ResourceRecord[] }) {
  if (!records || records.length === 0) {
    return <div className="text-sm text-gray-500">No resource records found.</div>;
  }

  return (
    <table className="w-full text-sm">
      <thead>
        <tr className="text-left text-gray-500 border-b">
          <th className="px-3 py-2">Type</th>
          <th className="px-3 py-2">Value</th>
        </tr>
      </thead>
      <tbody>
        {records.map((rec, i) => (
          <tr key={i} className="border-t border-gray-100">
            <td className="px-3 py-2 font-mono font-semibold">{rec.type}</td>
            <td className="px-3 py-2 font-mono text-xs break-all">
              {rec.type === "NS" && rec.ns}
              {rec.type === "GLUE4" && `${rec.ns} → ${rec.address}`}
              {rec.type === "GLUE6" && `${rec.ns} → ${rec.address}`}
              {rec.type === "TXT" && rec.txt?.join(" ")}
              {rec.type === "DS" && `${rec.hash} ${rec.algorithm} ${rec.digestType} ${rec.digest}`}
              {rec.type === "SYNTH4" && rec.address}
              {rec.type === "SYNTH6" && rec.address}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function DnsRecords() {
  const { data: assets = [] } = useAssets({});
  const [selectedName, setSelectedName] = useState("");

  const { data: nameInfo, isLoading, refetch, error } = useQuery<NameInfo>({
    queryKey: ["dns", selectedName],
    queryFn: () => invoke<NameInfo>("get_resource", { name: selectedName }),
    enabled: false,
  });

  const ownedAssets = assets.filter((a) => a.status === "finalized_owned");

  return (
    <div className="space-y-4">
      <h2 className="text-xl font-bold">DNS Records</h2>

      <div className="bg-white rounded p-4 border border-gray-200 text-sm text-gray-600">
        View DNS resource records for names owned by your wallet. Read-only in MVP.
      </div>

      <div className="flex gap-3 items-end">
        <Select
          label="Select Name"
          options={[
            { value: "", label: "-- Select --" },
            ...ownedAssets.map((a) => ({ value: a.tld, label: `.${a.tld}` })),
          ]}
          value={selectedName}
          onChange={(e) => setSelectedName(e.target.value)}
        />
        <Button
          onClick={() => refetch()}
          disabled={!selectedName || isLoading}
        >
          {isLoading ? "Loading..." : "Fetch Records"}
        </Button>
      </div>

      {error && (
        <div className="bg-red-50 rounded p-4 border border-red-200 text-red-700 text-sm">
          Failed to fetch records. Make sure the name is owned by your wallet and the connection is active.
        </div>
      )}

      {nameInfo && (
        <div className="space-y-4">
          <div className="bg-white rounded p-4 border border-gray-200">
            <h3 className="text-sm font-semibold mb-2">.{selectedName}</h3>
            <div className="grid grid-cols-3 gap-4 mb-4">
              <div>
                <div className="text-xs text-gray-500">State</div>
                <div className="text-sm font-medium">{nameInfo.state || "—"}</div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Height</div>
                <div className="text-sm font-mono">{nameInfo.height ? `#${nameInfo.height}` : "—"}</div>
              </div>
              <div>
                <div className="text-xs text-gray-500">Days Until Expire</div>
                <div className="text-sm font-mono">
                  {nameInfo.stats?.daysUntilExpire ? `${Math.round(nameInfo.stats.daysUntilExpire)}d` : "—"}
                </div>
              </div>
            </div>

            <h4 className="text-xs font-semibold text-gray-500 mb-2">Resource Records</h4>
            {nameInfo.data?.records ? (
              <RecordTable records={nameInfo.data.records} />
            ) : (
              <div className="text-sm text-gray-400">No records data available.</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
