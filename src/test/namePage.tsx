import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { NameRoute } from "../components/NamePage";

/** The current path + query, for asserting navigation. */
export function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="location">{loc.pathname + loc.search}</div>;
}

/**
 * The name page as the app routes it, at `path` (e.g. "/names/foo?tab=dns").
 * Every other path renders nothing, so a test can assert where it went.
 */
export function NamePageAt({ path }: { path: string }) {
  return (
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/names/:name" element={<NameRoute />} />
        <Route path="*" element={null} />
      </Routes>
      <LocationProbe />
    </MemoryRouter>
  );
}
