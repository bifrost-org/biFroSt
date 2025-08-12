import { z } from "zod";

const usernameRegex = /^[a-zA-Z][a-zA-Z0-9_-]{0,31}$/;

export const registrationSchema = z
  .object({
    username: z.string().regex(usernameRegex, {
      message:
        "Username must start with a letter and contain only letters, numbers, hyphens or underscores. Max 32 characters.",
    }),
  })
  .strict();

export type RegistrationPost = z.infer<typeof registrationSchema>;
