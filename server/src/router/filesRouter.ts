import { Router, Request, Response, NextFunction } from "express";
import { StatusCodes } from "http-status-codes";
import fs from "fs/promises";
import path from "path";
import { FileAttr, FileType, getNodeType, Mode } from "../model/file";
import { FileError } from "../error/fileError";
import {
  validateMultipartMetadata,
  validatePathParameter,
} from "../middleware/validation";
import { MetadataPut } from "../validation/metadataSchema";

export const filesRouter: Router = Router();

const USER_PATH = process.env.USERS_PATH;

function getPath(a: string | undefined, b: string): string {
  return path.join(a!, b);
}

// GET /files/:path
filesRouter.get(
  "/files/:path?",
  validatePathParameter(false),
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      const filePath = getPath(USER_PATH, req.params.path);
      const fileContent = await fs.readFile(filePath);
      res.status(StatusCodes.OK).send(fileContent);
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        next(FileError.NotFound());
      } else {
        next(e);
      }
    }
  }
);

// PUT /files/:path
filesRouter.put(
  "/files/:path?",
  validatePathParameter(false),
  validateMultipartMetadata,
  async (req: Request, res: Response, next: NextFunction) => {
    const currentPath = req.params.path;
    const { metadata, content } = req.body as {
      metadata: MetadataPut;
      content?: { path: string };
    };

    const finalPath = getPath(USER_PATH, metadata.newPath || currentPath);

    try {
      if (
        (metadata.kind === FileType.SymLink ||
          metadata.kind === FileType.HardLink) &&
        metadata.refPath
      ) {
        const linkTarget = getPath(USER_PATH, metadata.refPath);
        if (metadata.kind === FileType.SymLink) {
          await fs.symlink(linkTarget, finalPath);
        } else {
          await fs.link(linkTarget, finalPath);
        }
        return res.status(StatusCodes.CREATED).send();
      }

      let contentBuffer: Buffer | undefined = undefined;
      if (content?.path && metadata.mode !== Mode.Truncate) {
        contentBuffer = await fs.readFile(content.path);

        if (contentBuffer.length !== metadata.size) {
          return next(FileError.SizeMismatch());
        }
      }

      const fileExists: boolean = await fs
        .access(finalPath)
        .then(() => true)
        .catch(() => false);

      switch (metadata.mode) {
        case Mode.Write:
          await fs.writeFile(finalPath, contentBuffer ?? Buffer.alloc(0));
          break;

        case Mode.Append:
          await fs.appendFile(finalPath, contentBuffer ?? Buffer.alloc(0));
          break;

        case Mode.WriteAt: {
          const fd = await fs.open(finalPath, fileExists ? "r+" : "w+");
          try {
            const buffer = contentBuffer ?? Buffer.alloc(0);
            await fd.write(buffer, 0, buffer.length, metadata.offset);
          } finally {
            await fd.close();
          }
          break;
        }

        case Mode.Truncate:
          await fs.truncate(finalPath, metadata.size);
          break;

        default:
        // this section cannot be accessed because zod intercept the error
      }

      await fs.chmod(finalPath, parseInt(metadata.perm, 8));
      await fs.utimes(
        finalPath,
        new Date(metadata.atime),
        new Date(metadata.mtime)
      );
      // NOTE: ctime and crtime are not manually settable. They are controlled by the file system

      // Delete old path if moved
      if (metadata.newPath && metadata.newPath !== currentPath) {
        const oldPath = getPath(USER_PATH, currentPath);
        await fs.rm(oldPath).catch(() => {});
      }

      const status = fileExists ? StatusCodes.NO_CONTENT : StatusCodes.CREATED;
      res.status(status).send();
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        console.log(e);
        next(FileError.ParentDirectoryNotFound());
      } else if (code === "EEXIST") {
        next(FileError.FileAlreadyExists());
      } else {
        next(e);
      }
    }
  }
);

// DELETE /files/:path
filesRouter.delete(
  "/files/:path?",
  validatePathParameter(false),
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      const filePath = getPath(USER_PATH, req.params.path);
      const stat = await fs.stat(filePath);

      if (stat.isDirectory()) {
        await fs.rmdir(filePath);
      } else {
        await fs.unlink(filePath);
      }

      res.status(StatusCodes.NO_CONTENT).send();
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        next(FileError.NotFound());
      } else if (code === "ENOTEMPTY") {
        next(FileError.DirectoryNotEmpty());
      } else {
        next(e);
      }
    }
  }
);

// GET /list/:path
filesRouter.get(
  "/list/:path?",
  validatePathParameter(true),
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      const entryPath = getPath(USER_PATH, req.params.path ?? "");

      const stats = await fs.stat(entryPath);

      // if the entry is a file, the output will be an array with a single object containing its metadata
      if (!stats.isDirectory()) {
        const parentDirPath = path.dirname(entryPath);
        const entries = await fs.readdir(parentDirPath, {
          withFileTypes: true,
        });
        const entry = entries.find((e) => e.name === path.basename(entryPath))!; // cannot be undefined
        const fsEntry: FileAttr = {
          name: entry.name,
          size: stats.size,
          atime: stats.atime.toISOString(),
          mtime: stats.mtime.toISOString(),
          ctime: stats.ctime.toISOString(),
          crtime: stats.birthtime.toISOString(),
          kind: getNodeType(entry),
          perm: (stats.mode & 0o777).toString(8), // octal mask to isolate permissions bits
          nlink: stats.nlink,
        };

        return res.status(StatusCodes.OK).json([fsEntry]);
      }

      const dirPath = entryPath; // the entry is now assumed to be a directory
      const entries = await fs.readdir(dirPath, { withFileTypes: true });

      const result = await Promise.all(
        entries.map(async (entry) => {
          const entryPath = getPath(dirPath, entry.name);
          const stats = await fs.stat(entryPath);
          return {
            name: entry.name,
            size: stats.size,
            atime: stats.atime.toISOString(),
            mtime: stats.mtime.toISOString(),
            ctime: stats.ctime.toISOString(),
            crtime: stats.birthtime.toISOString(),
            kind: getNodeType(entry),
            perm: (stats.mode & 0o777).toString(8),
            nlink: stats.nlink,
          } satisfies FileAttr;
        })
      );

      res.status(StatusCodes.OK).json(result);
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        next(FileError.NotFound());
      } else {
        next(e);
      }
    }
  }
);

// POST /mkdir/:path
filesRouter.post(
  "/mkdir/:path?",
  validatePathParameter(false),
  async (req: Request, res: Response, next: NextFunction) => {
    try {
      const dirPath = getPath(USER_PATH, req.params.path);

      await fs.mkdir(dirPath);
      res.status(StatusCodes.CREATED).send();
    } catch (e) {
      const code = (e as NodeJS.ErrnoException).code;
      if (code === "ENOENT") {
        next(FileError.ParentDirectoryNotFound());
      } else if (code === "EEXIST") {
        next(FileError.DirectoryAlreadyExists());
      } else {
        next(e);
      }
    }
  }
);
