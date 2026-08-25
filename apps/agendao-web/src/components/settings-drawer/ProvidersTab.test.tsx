import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SetStateAction } from "react";
import type { ManagedModelOverrideInfoRecord, ManagedProviderInfoRecord } from "@/lib/provider";
import { apiJson } from "@/lib/api";
import type { ModelOverrideDraft } from "./types";
import { ProvidersTab } from "./ProvidersTab";

vi.mock("@/lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...original,
    apiJson: vi.fn<(path: string, options?: RequestInit) => Promise<unknown>>(),
  };
});

const apiJsonMock = vi.mocked(apiJson);

const managedProviders: ManagedProviderInfoRecord[] = [
  {
    id: "openai",
    name: "OpenAI",
    status: "connected",
    connected: true,
    configured: true,
    known: true,
    has_auth: true,
  },
  {
    id: "anthropic",
    name: "Anthropic",
    status: "connected",
    connected: true,
    configured: true,
    known: true,
    has_auth: true,
    disabled: true,
  },
];

const emptyDraft: ModelOverrideDraft = {
  providerId: "",
  modelKey: "",
  modelId: "",
  name: "",
  baseUrl: "",
  family: "",
  status: "",
  releaseDate: "",
  reasoning: false,
  reasoningEffort: "",
  toolCall: false,
  attachment: false,
  temperature: false,
  experimental: false,
};

function renderProvidersTab(overrides: Partial<Parameters<typeof ProvidersTab>[0]> = {}) {
  const props: Parameters<typeof ProvidersTab>[0] = {
    styles: {
      primaryButtonClass: "primary",
      secondaryButtonClass: "secondary",
      formFieldClass: "field",
      formLabelClass: "label",
      formHintClass: "hint",
      inputClass: "input",
      selectClass: "select",
      checkboxRowClass: "checkbox-row",
      checkboxClass: "checkbox",
    },
    busyKey: null,
    providers: [],
    providerSummary: "0 configured",
    connectProtocols: [],
    connectQuery: "",
    onConnectQueryChange: vi.fn<(value: string) => void>(),
    connectResolution: null,
    connectResolveBusy: false,
    connectResolveError: null,
    connectProviderId: "",
    onConnectProviderIdChange: vi.fn<(value: string) => void>(),
    connectProtocol: "",
    onConnectProtocolChange: vi.fn<(value: string) => void>(),
    connectApiKey: "",
    onConnectApiKeyChange: vi.fn<(value: string) => void>(),
    connectBaseUrl: "",
    onConnectBaseUrlChange: vi.fn<(value: string) => void>(),
    connectBusy: false,
    onConnectProvider: vi.fn<() => Promise<void>>().mockResolvedValue(undefined),
    onReloadSettingsData: vi.fn<() => Promise<void>>().mockResolvedValue(undefined),
    onRemoveProvider: vi.fn<(providerId: string) => void>(),
    onToggleProviderDisabled: vi.fn<(providerId: string, disabled: boolean) => void>(),
    onRenameProvider: vi.fn<(providerId: string, name: string) => void>(),
    onRefreshProviderCatalogue: vi.fn<() => void>(),
    managedProviders,
    selectedManagedProviderId: null,
    onSelectedManagedProviderIdChange: vi.fn<(value: string) => void>(),
    providerDescriptorLoading: false,
    selectedProviderDescriptor: null,
    selectedProviderDescriptorError: null,
    modelOverrideDraft: emptyDraft,
    onModelOverrideDraftChange: vi.fn<(value: SetStateAction<ModelOverrideDraft>) => void>(),
    editingModelTarget: null,
    modelOverrideProviderOptions: [],
    configuredModelOverrides: [],
    onResetModelOverrideDraft: vi.fn<(providerId?: string) => void>(),
    onEditModelOverride: vi.fn<
      (providerId: string, record: ManagedModelOverrideInfoRecord) => void
    >(),
    onSaveModelOverride: vi.fn<() => void>(),
    onDeleteModelOverride: vi.fn<(providerId: string, modelKey: string) => void>(),
    ...overrides,
  };
  render(<ProvidersTab {...props} />);
  return props;
}

describe("ProvidersTab managed provider toggle", () => {
  beforeEach(() => {
    apiJsonMock.mockReset();
  });

  it("derives disabled badge and toggle label from the disabled flag", () => {
    renderProvidersTab();

    const enabledRow = screen.getByTestId("settings-managed-provider-row-openai");
    expect(enabledRow).toHaveAttribute("data-disabled", "false");
    expect(screen.queryByTestId("settings-provider-disabled-badge-openai")).toBeNull();
    expect(screen.getByTestId("settings-provider-toggle-openai")).toHaveTextContent("Disable");

    const disabledRow = screen.getByTestId("settings-managed-provider-row-anthropic");
    expect(disabledRow).toHaveAttribute("data-disabled", "true");
    expect(screen.getByTestId("settings-provider-disabled-badge-anthropic")).toHaveTextContent(
      "Disabled",
    );
    expect(screen.getByTestId("settings-provider-toggle-anthropic")).toHaveTextContent("Enable");
  });

  it("calls onToggleProviderDisabled with the inverted disabled flag", () => {
    const props = renderProvidersTab();

    fireEvent.click(screen.getByTestId("settings-provider-toggle-openai"));
    expect(props.onToggleProviderDisabled).toHaveBeenCalledWith("openai", true);

    fireEvent.click(screen.getByTestId("settings-provider-toggle-anthropic"));
    expect(props.onToggleProviderDisabled).toHaveBeenCalledWith("anthropic", false);
  });
});

describe("ProvidersTab test connection", () => {
  it("derives inline success and failure results from the test endpoint", async () => {
    apiJsonMock.mockResolvedValueOnce({ ok: true, status: 200, latency_ms: 123 });
    renderProvidersTab();

    fireEvent.click(screen.getByTestId("settings-provider-test-openai"));
    const okResult = await screen.findByTestId("settings-provider-test-result-openai");
    expect(okResult).toHaveTextContent("✓ 200 · 123ms");
    expect(okResult.className).toContain("text-(--ds-ok)");
    expect(apiJsonMock).toHaveBeenCalledWith("/provider/openai/test", { method: "POST" });

    apiJsonMock.mockResolvedValueOnce({ ok: false, latency_ms: 4, error: "connection refused" });
    fireEvent.click(screen.getByTestId("settings-provider-test-anthropic"));
    const failedResult = await screen.findByTestId("settings-provider-test-result-anthropic");
    expect(failedResult).toHaveTextContent("✗ connection refused");
    expect(failedResult.className).toContain("text-(--ds-error)");
  });
});

describe("ProvidersTab provider rename", () => {
  it("renames inline on Enter and cancels on Escape", () => {
    const props = renderProvidersTab();

    fireEvent.click(screen.getByTestId("settings-provider-rename-openai"));
    const input = screen.getByTestId("settings-provider-rename-input-openai");
    expect(input).toHaveValue("OpenAI");

    fireEvent.change(input, { target: { value: "OpenAI Prod" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(props.onRenameProvider).toHaveBeenCalledTimes(1);
    expect(props.onRenameProvider).toHaveBeenCalledWith("openai", "OpenAI Prod");
    expect(screen.queryByTestId("settings-provider-rename-input-openai")).toBeNull();

    fireEvent.click(screen.getByTestId("settings-provider-rename-anthropic"));
    const cancelled = screen.getByTestId("settings-provider-rename-input-anthropic");
    fireEvent.change(cancelled, { target: { value: "Discarded" } });
    fireEvent.keyDown(cancelled, { key: "Escape" });
    expect(props.onRenameProvider).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("settings-provider-rename-input-anthropic")).toBeNull();
  });
});
