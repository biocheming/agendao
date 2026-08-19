"use client";

import { cn } from "@/lib/utils";
import { Suspense, lazy, memo } from "react";

const StreamdownRenderer = lazy(async () => {
  const module = await import("./streamdown-runtime");
  return { default: module.StreamdownRenderer };
});

export interface MessageResponseProps {
  children: string;
  className?: string;
}

function MessageResponseFallback({
  children,
  className,
}: MessageResponseProps) {
  return (
    <div className={cn("size-full whitespace-pre-wrap break-words", className)}>
      {children}
    </div>
  );
}

export const MessageResponse = memo(
  ({ className, ...props }: MessageResponseProps) => (
    <Suspense
      fallback={
        <MessageResponseFallback
          className={cn(
            "size-full [&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
            className
          )}
          {...props}
        />
      }
    >
      <StreamdownRenderer
        className={cn(
          "size-full [&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
          className
        )}
        unsafeLinks
        {...props}
      />
    </Suspense>
  ),
  (prevProps, nextProps) => prevProps.children === nextProps.children
);

MessageResponse.displayName = "MessageResponse";
