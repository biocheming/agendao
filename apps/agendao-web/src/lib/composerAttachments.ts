import {
  defaultAttachmentUploadPath,
  fileUrlFromPath,
  preparePromptPartsFromFiles,
  readFilesAsPromptParts,
  type FilePromptPart,
} from "./composerContext";

interface UploadResponse {
  files: Array<{
    name: string;
    path: string;
    mime?: string;
  }>;
}

export async function prepareComposerAttachments(
  files: File[],
  options: {
    workspaceBasePath: string;
    uploadJson: <T>(path: string, init: RequestInit) => Promise<T>;
  },
): Promise<FilePromptPart[]> {
  const uploadPath = defaultAttachmentUploadPath(options.workspaceBasePath);

  const uploadLargeFile = async (file: File): Promise<FilePromptPart> => {
    const [part] = await readFilesAsPromptParts([file]);
    const response = await options.uploadJson<UploadResponse>("/file/upload", {
      method: "POST",
      body: JSON.stringify({
        path: uploadPath,
        files: [
          {
            name: file.name,
            content: part.url,
            mime: file.type || undefined,
          },
        ],
      }),
    });

    const uploaded = response.files[0];
    if (!uploaded) {
      throw new Error("Upload returned no files");
    }

    return {
      type: "file",
      url: fileUrlFromPath(uploaded.path),
      filename: uploaded.name,
      mime: file.type || uploaded.mime || undefined,
    };
  };

  return preparePromptPartsFromFiles(files, {
    uploadLargeFile,
  });
}
