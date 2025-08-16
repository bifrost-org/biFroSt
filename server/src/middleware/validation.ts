import { Request, Response, NextFunction } from "express";
import { StatusCodes } from "http-status-codes";
import { ZodSchema } from "zod";
import multiparty from "multiparty";
import { metadataSchema } from "../validation/metadataSchema";

const createValidator =
  (targetName: "body" | "params" | "query") =>
  (schema: ZodSchema) =>
  (req: Request, res: Response, next: NextFunction) => {
    const parseResult = schema.safeParse(req[targetName]);
    if (!parseResult.success) {
      return res.status(StatusCodes.BAD_REQUEST).json(parseResult.error); // Error 400
    }
    next();
  };

export const validateBody = createValidator("body");
export const validateRequestParameters = createValidator("params");
export const validateQueryParameters = createValidator("query");

export const validateMultipartMetadata = (
  req: Request,
  res: Response,
  next: NextFunction
) => {
  const form = new multiparty.Form();

  form.parse(req, (err, fields, files) => {
    if (err) {
      return res
        .status(StatusCodes.BAD_REQUEST)
        .json({ error: "Invalid multipart data" });
    }

    if (!fields.metadata || !fields.metadata[0]) {
      return res
        .status(StatusCodes.BAD_REQUEST)
        .json({ error: "Missing metadata field" });
    }

    let metadataParsed: object;
    try {
      metadataParsed = JSON.parse(fields.metadata[0]);
    } catch {
      return res
        .status(StatusCodes.BAD_REQUEST)
        .json({ error: "Malformed metadata JSON" });
    }

    const validation = metadataSchema.safeParse(metadataParsed);

    if (!validation.success) {
      return res.status(StatusCodes.BAD_REQUEST).json({
        error: "Invalid metadata",
        details: validation.error.errors,
      });
    }

    req.body = {
      originalMetadata: metadataParsed,
      metadata: validation.data,
      content: files.content ? files.content[0] : undefined,
    };

    next();
  });
};

import { FileError } from "../error/fileError";

export function validatePathParameter(allowEmpty = false) {
  return (req: Request, res: Response, next: NextFunction) => {
    const path = req.params.path;
    if (
      (!path && !allowEmpty) || // error if path not present but empty path not allowed
      (path && path.includes("..")) // error if path present but path includes the string ".."
    ) {
      return next(FileError.InvalidPath());
    }
    next();
  };
}
