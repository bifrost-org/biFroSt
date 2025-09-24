import { z } from "zod";
import { FileType, Mode } from "../model/file";

const filePathPattern = /^([^\0\/:*?"<>|]+\/)*[^\0\/:*?"<>|]+$/;

const baseMetadataSchema = z.object({
  newPath: z
    .string()
    .regex(filePathPattern, {
      message: "newPath must be a valid path like /folder/file.txt",
    })
    .optional(),

  size: z.number().int().nonnegative(),

  atime: z.string().datetime(),
  mtime: z.string().datetime(),

  kind: z.nativeEnum(FileType),
  refPath: z.string().optional(),

  perm: z.string().regex(/^[0-7]{3}$/),

  mode: z.nativeEnum(Mode),
  offset: z.number().int().nonnegative().optional(),
});

export const metadataSchema = baseMetadataSchema.superRefine(
  (metadata, ctx) => {
    if (metadata.kind === FileType.Directory) {
      ctx.addIssue({
        path: ["kind"],
        code: z.ZodIssueCode.custom,
        message: "`directory` is not allowed as kind for this endpoint",
      });
    }

    if (
      (metadata.kind === FileType.SymLink ||
        metadata.kind === FileType.HardLink) &&
      !metadata.refPath
    ) {
      ctx.addIssue({
        path: ["refPath"],
        code: z.ZodIssueCode.custom,
        message: "refPath is required when kind is 'soft_link' or 'hard_link'",
      });
    }

    if (metadata.mode === Mode.WriteAt && !metadata.offset) {
      ctx.addIssue({
        path: ["offset"],
        code: z.ZodIssueCode.custom,
        message: "offset is required when mode is 'write_at'",
      });
    }
  }
);

export type MetadataPut = z.infer<typeof metadataSchema>;
