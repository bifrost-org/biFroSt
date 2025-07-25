import { Router, Request, Response, NextFunction } from "express";
import { StatusCodes } from "http-status-codes";
import fs from "fs/promises";
import fsSync from "fs";
import multiparty from "multiparty";
import path from "path";
import { FileError } from "../error/filesError";

export const filesRouter: Router = Router();

const USER_PATH = process.env.USER_PATH;

console.log(USER_PATH);

// GET /files/:path
filesRouter.get(
  "/files/:path",
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      const filePath = `${USER_PATH}${req.params.path}`;
      const content = await fs.readFile(filePath);
      res.status(StatusCodes.OK).send(content);
    } catch {
      next(FileError.NotFound());
    }
  }
);

// PUT /files/:path
filesRouter.put("/files/:path", async (req: Request, res: Response) => {
  const currentPath = req.params.path;

  try {
    const form = new multiparty.Form();
    form.parse(req, async (err, fields, files) => {
      if (err || !fields || !files || !files.content) {
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Invalid multipart data" });
        return;
      }

      let metadata;
      try {
        metadata = JSON.parse(fields.metadata?.[0] || "{}");
      } catch {
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Malformed metadata JSON" });
        return;
      }

      const { newPath, size, permissions, lastModified } = metadata;
      if (!size || !permissions || !lastModified) {
        // TODO: make this a FileError
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Missing required metadata fields" });
        return;
      }

      const finalPath = `${USER_PATH}${newPath || currentPath}`;

      const contentFile = files.content[0];
      console.log(contentFile.path);
      const fileBuffer = await fs.readFile(contentFile.path);

      // Verify integrity
      if (fileBuffer.length !== size) {
        res
          .status(StatusCodes.BAD_REQUEST) // TODO: make this a FileError
          .send({ error: "Size mismatch: integrity verification failed" });
        return;
      }

      await fs.mkdir(path.dirname(finalPath), { recursive: true });

      let fileAlreadyExists = false;
      try {
        await fs.access(finalPath);
        fileAlreadyExists = true;
      } catch {
        /* empty */
      }

      await fs.writeFile(finalPath, fileBuffer);

      // Attempt to set last modified timestamp // TODO: this should work when there are sync problems
      /*
      try {
        await fs.utimes(finalPath, new Date(), new Date(modified));
      } catch {
        // best-effort only
      }
        */

      // Handle move: remove original if path differs
      if (newPath && newPath !== currentPath) {
        const oldPath = `${USER_PATH}${currentPath}`;
        if (oldPath !== finalPath) {
          try {
            await fs.rm(oldPath);
          } catch {
            // file may already have been overwritten
          }
        }
        res.status(StatusCodes.NO_CONTENT).send(); // If the file was moved to a new path or its content was updated
      } else {
        const status = fileAlreadyExists
          ? StatusCodes.NO_CONTENT
          : StatusCodes.CREATED;
        res.status(status).send();
      }
    });
  } catch (error) {
    res
      .status(StatusCodes.INTERNAL_SERVER_ERROR) // TODO: Make this a FileError
      .send({ error: "Unable to write file", description: error });
  }
});

// DELETE /files/:path
filesRouter.delete(
  "/files/:path",
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      const filePath = `${USER_PATH}${req.params.path}`;
      const stat = await fs.stat(filePath);
      if (stat.isDirectory()) {
        await fs.rm(filePath, { recursive: true, force: true });
      } else {
        await fs.unlink(filePath);
      }
      res.status(StatusCodes.NO_CONTENT).send();
    } catch {
      next(FileError.NotFound());
    }
  }
);

// GET /list/:path
// TODO: make this work for root entries. Maybe a special path
filesRouter.get("/list/:path", async (req: Request, res: Response) => {
  try {
    const dirPath = `${USER_PATH}${req.params.path}`;
    const stat = await fs.stat(dirPath);
    if (!stat.isDirectory()) {
      res
        .status(StatusCodes.BAD_REQUEST) // TODO: make this a FileError
        .send({ error: "Path is not a directory" });
      return;
    }

    const entries = await fs.readdir(dirPath, { withFileTypes: true });

    const result = await Promise.all(
      entries.map(async (entry) => {
        const entryPath = path.join(dirPath, entry.name);
        const stats = await fs.stat(entryPath);
        return {
          name: entry.name,
          isDirectory: entry.isDirectory(),
          size: entry.isDirectory() ? 0 : stats.size,
          permissions: fsSync.existsSync(entryPath) // TODO: check this part
            ? fsSync.statSync(entryPath).mode.toString(8).slice(-3) // mock POSIX-style
            : "rw-",
          lastModified: stats.mtime.toISOString(),
        };
      })
    );

    res.status(StatusCodes.OK).json(result);
  } catch (err: unknown) {
    // TODO: make these errors into FileErrors
    if (
      typeof err === "object" &&
      err !== null &&
      "code" in err &&
      err.code === "ENOENT"
    ) {
      res.status(StatusCodes.NOT_FOUND).send({ error: "Directory not found" });
    } else if (
      typeof err === "object" &&
      err !== null &&
      "message" in err &&
      typeof err.message === "string" &&
      err.message.includes("Access denied")
    ) {
      res.status(StatusCodes.BAD_REQUEST).send({ error: "Path is malformed" });
    } else {
      res.status(StatusCodes.INTERNAL_SERVER_ERROR).send({
        error: "Unexpected error",
        description:
          typeof err === "object" && err !== null && "message" in err
            ? err.message
            : String(err),
      });
    }
  }
});

// POST /mkdir/:path
filesRouter.post("/mkdir/:path", async (req: Request, res: Response) => {
  try {
    const dirPath = `${USER_PATH}${req.params.path}`;

    try {
      await fs.mkdir(dirPath);
      res.status(StatusCodes.CREATED).send();
    } catch (err: any) {
      // TODO: make these FileError
      if (err.code === "EEXIST") {
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Directory already exists" });
      } else if (err.code === "ENOENT") {
        res
          .status(StatusCodes.BAD_REQUEST)
          .send({ error: "Parent directory does not exist" });
      } else {
        res
          .status(StatusCodes.INTERNAL_SERVER_ERROR)
          .send({ error: "Internal server error", description: err.message });
      }
    }
  } catch {
    res
      .status(StatusCodes.BAD_REQUEST)
      .send({ error: "Invalid or unsafe path" });
  }
});
